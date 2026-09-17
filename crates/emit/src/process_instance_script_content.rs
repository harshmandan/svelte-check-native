//! Hoist module-level statements out of an instance script body.
//!
//! `<script>` content in a Svelte 5 component is module-scope code, but our
//! emit wraps it in `function $$render() { ... }`. Several statement kinds
//! are illegal inside a function body and must be lifted to module top
//! level:
//!
//! - **`import`** — TS1232 if inside a function
//! - **`export const/let/var/function/class`** — TS1184 / TS1233
//! - **`export { a, b }` / `export { a as b }`** — TS1233
//! - **`export { a } from 'mod'`** — TS1233
//! - **`export * from 'mod'`** — TS1232
//!
//! All are hoisted to a module-level prelude. The original spans inside
//! the script body are blanked with whitespace of the same *byte* length so
//! byte offsets inside the body stay aligned for source-map mapping. Column
//! (UTF-16) positions also stay aligned for ASCII content; a blanked
//! multibyte char expands to len_utf8() spaces, so columns of any live code
//! that follows it on the same physical line drift right by one per multibyte
//! char (latent — blanked spans are whole statements, rarely followed by live
//! code on the same line).

use std::collections::HashSet;

use oxc_allocator::Allocator;
use oxc_ast::ast::{BindingPattern, Declaration, ImportOrExportKind, Statement};
use oxc_span::GetSpan;
use smol_str::SmolStr;
use svn_parser::{ScriptLang, parse_script_body};

use crate::svelte2tsx_nodes::exported_type_info::collect_export_type_infos;
use crate::svelte2tsx_nodes::hoistable_interfaces::{HoistContext, hoisted_type_spans};

/// `hoisted`: statements lifted to module top level (newline-joined).
/// `body`: the original script content with hoisted spans blanked out.
/// `exported_locals`: names that were `export`-ed in the source but
/// whose `export` keyword was stripped (the declaration stays in body).
/// Emit voids these so TS6133 doesn't flag them as unused — the user
/// declared them as public surface.
/// `hoisted_byte_offsets`: byte offsets into the original *content*
/// where each hoisted statement starts. Caller uses these to build a
/// line map so diagnostics inside hoisted regions point at the correct
/// source line, not at line 1.
#[derive(Debug, Clone)]
pub struct SplitScript {
    pub hoisted: String,
    pub body: String,
    pub exported_locals: Vec<SmolStr>,
    pub hoisted_byte_offsets: Vec<u32>,
    /// Byte span of each hoisted statement INSIDE `hoisted` (trailing
    /// newline included), parallel to `hoisted_byte_offsets`. The
    /// emit-side line-map builder walks these spans verbatim instead of
    /// re-deriving statement boundaries from the concatenated text —
    /// boundary heuristics broke on indented statements, collapsing the
    /// whole region into one entry anchored at the first import, which
    /// shifted every diagnostic after a blank source line by one.
    pub hoisted_stmt_spans: Vec<(usize, usize)>,
    /// Per-export type info for assembling the component's Exports
    /// intersection on the default-export type alias. `type_source` is
    /// `None` when the user didn't annotate the declaration (or when
    /// we couldn't extract a safe-to-hoist annotation); the caller
    /// falls back to `any` for those slots.
    pub export_type_infos: Vec<ExportedLocalInfo>,
}

/// Surface-facing type info for one `export function/let/const` — what
/// consumers of `bind:this={x}` see as `x.name` instance access.
#[derive(Debug, Clone)]
pub struct ExportedLocalInfo {
    pub name: SmolStr,
    pub type_source: Option<String>,
    /// `true` for `export let X` (Svelte-4 prop), `false` for
    /// `export const`, `export function`, `export class`. Mirrors
    /// upstream svelte2tsx's `isLet` flag in `ExportedNames`. Drives
    /// the `__svn_ensure_right_props<{...}>(... as $$Props)` lets-
    /// shape inclusion in `render_function::emit_render_body_return`.
    pub is_let: bool,
    /// `true` when the declarator has an initializer
    /// (`export let foo = 'bar'`). Drives the optional-marker
    /// decision in the lets-shape: a let with an initializer
    /// becomes `foo?: …`, a let without becomes `foo: …` (required).
    /// Always `true` for `export function` / `export class` (they're
    /// always defined at declaration site).
    pub has_init: bool,
    /// The name the component exposes, when `export { name as other }`
    /// renames it.
    pub exported_as: Option<SmolStr>,
    /// Came from an `export { … }` list rather than an exported
    /// declaration.
    pub is_named_export: bool,
}

/// Where an import's leading comments begin: the first comment between
/// the end of the previous statement (`trivia_start`) and the import.
/// Upstream moves every one of those comments along with the import
/// (`moveNode`), whatever its kind or line, so a `@ts-ignore` written
/// above or before an import keeps applying to it at module scope.
fn leading_comments_start(
    comments: &[oxc_ast::Comment],
    trivia_start: usize,
    span_start: usize,
) -> usize {
    comments
        .iter()
        .filter(|c| c.span.start as usize >= trivia_start && c.span.end as usize <= span_start)
        .map(|c| c.span.start as usize)
        .min()
        .unwrap_or(span_start)
}

/// Split out every module-level statement (imports, exports of all
/// shapes) from a script body, and move the type declarations
/// `hoist` says belong at module scope.
///
/// Re-parses the body once with oxc. If parsing panics on malformed user
/// code, the content is passed through unchanged.
pub fn split_imports(content: &str, _lang: ScriptLang, hoist: &HoistContext) -> SplitScript {
    // Always parse as TypeScript — TS is a superset of JS for our
    // purposes (we're identifying statement spans, not generating
    // runtime code). Parsing as TS lets us correctly handle scripts
    // that use type annotations even when `<script>` doesn't carry
    // `lang="ts"`. (Svelte 5 + svelte:options runes accepts this.)
    let allocator = Allocator::default();
    let parsed = parse_script_body(&allocator, content, ScriptLang::Ts);

    if parsed.panicked {
        return SplitScript {
            hoisted: String::new(),
            body: content.to_string(),
            exported_locals: Vec::new(),
            hoisted_byte_offsets: Vec::new(),
            hoisted_stmt_spans: Vec::new(),
            export_type_infos: Vec::new(),
        };
    }

    // Spans we hoist verbatim to module top level: imports,
    // `export { x } from 'mod'`, `export * from 'mod'`, and the type
    // declarations `hoistable_interfaces` moves.
    let mut hoist_spans: Vec<(usize, usize)> = Vec::new();
    // Spans where we strip just the `export ` prefix and let the inner
    // declaration stay in the body. For `export const/let/var/function/class`
    // — the declaration body might reference locals (e.g. `export function
    // getA() { return a; }` where `a` is a local), so hoisting would
    // break those references.
    let mut strip_keyword_spans: Vec<(usize, usize)> = Vec::new();
    // Spans we drop entirely (blank in body, don't add to hoisted prelude).
    // For `export { x, y }` (no `from`) re-exports of local names.
    let mut drop_spans: Vec<(usize, usize)> = Vec::new();
    let mut exported_locals: Vec<SmolStr> = Vec::new();
    let mut export_type_infos: Vec<ExportedLocalInfo> = Vec::new();
    // `export { local as exported }` entries, resolved once every
    // top-level `let`/`var` is known.
    let mut named_exports: Vec<(SmolStr, SmolStr)> = Vec::new();
    let mut top_level_lets: HashSet<SmolStr> = HashSet::new();

    let mut prev_stmt_end = 0usize;
    for stmt in &parsed.program.body {
        let trivia_start = prev_stmt_end;
        prev_stmt_end = oxc_span::GetSpan::span(stmt).end as usize;
        match stmt {
            Statement::ImportDeclaration(decl) => {
                let start = leading_comments_start(
                    &parsed.program.comments,
                    trivia_start,
                    decl.span.start as usize,
                );
                hoist_spans.push((start, decl.span.end as usize));
            }
            Statement::VariableDeclaration(decl) => {
                // Body-level `let`/`var` — stays in body; remembered so an
                // `export { name }` list can tell a prop from a constant.
                if matches!(
                    decl.kind,
                    oxc_ast::ast::VariableDeclarationKind::Let
                        | oxc_ast::ast::VariableDeclarationKind::Var
                ) {
                    let mut names = Vec::new();
                    for d in &decl.declarations {
                        collect_binding_pattern_names(&d.id, &mut names);
                    }
                    top_level_lets.extend(names);
                }
            }
            Statement::FunctionDeclaration(_) | Statement::ClassDeclaration(_) => {}
            // `export const/type/function/...` — one declaration, no
            // specifiers. Its own statement kind since oxc 0.143.
            Statement::ExportDeclaration(decl) => {
                let span = (decl.span.start as usize, decl.span.end as usize);
                {
                    let d = &decl.declaration;
                    // `export type Foo = ...` / `export interface Foo { ... }`
                    // are left exactly as written unless the hoisting pass
                    // below moves them: in the render function the
                    // `export` modifier is an error, as it is upstream.
                    if matches!(
                        d,
                        Declaration::TSTypeAliasDeclaration(_)
                            | Declaration::TSInterfaceDeclaration(_)
                    ) {
                        continue;
                    }
                    // `export const/let/var/function/class` — strip just
                    // the `export ` prefix. The declaration content stays
                    // in body where its identifier references resolve.
                    let inner_start = GetSpan::span(d).start as usize;
                    if inner_start > span.0 {
                        strip_keyword_spans.push((span.0, inner_start));
                    }
                    collect_declaration_names(d, &mut exported_locals);
                    collect_export_type_infos(d, content, &mut export_type_infos);
                }
            }
            // `export { x } from 'mod'` — pure module re-export, no
            // local name references. Hoist.
            Statement::ExportFromDeclaration(decl) => {
                hoist_spans.push((decl.span.start as usize, decl.span.end as usize));
            }
            Statement::ExportNamedDeclaration(decl) => {
                let span = (decl.span.start as usize, decl.span.end as usize);
                {
                    // `export { x, y }` (no `from`) — local name re-export.
                    // Drop the statement, but the names ARE exported, so
                    // record them for void-emission.
                    //
                    // Type-only specifiers (`export { type Bar }` or a
                    // whole-decl `export type { Bar }`) must NOT be added
                    // to `exported_locals`: the emit wraps each entry in
                    // `void <name>;`, and voiding a type name fires TS2693
                    // ("'Bar' only refers to a type but is being used as a
                    // value here"). Types don't need void'ing for TS6133
                    // anyway — they aren't emitted at runtime.
                    drop_spans.push(span);
                    let decl_type_only = decl.export_kind == ImportOrExportKind::Type;
                    for spec in &decl.specifiers {
                        if decl_type_only || spec.export_kind == ImportOrExportKind::Type {
                            continue;
                        }
                        let local = SmolStr::from(spec.local.name().as_str());
                        exported_locals.push(local.clone());
                        named_exports.push((local, SmolStr::from(spec.exported.name().as_str())));
                    }
                }
            }
            // `export default …` stays in the render function as written,
            // where it is an error (TS1258), as upstream leaves it.
            Statement::ExportDefaultDeclaration(_) => {}
            Statement::ExportAllDeclaration(decl) => {
                hoist_spans.push((decl.span.start as usize, decl.span.end as usize));
            }
            // Namespaces, `declare module`, and top-level `type` /
            // `interface` declarations stay in the body unless the
            // hoisting pass below (`hoistable_interfaces`) moves them. A
            // namespace is invalid inside a function, so TS1235 fires for
            // one written here, as upstream lets it.
            Statement::TSNamespaceDeclaration(_)
            | Statement::TSExternalModuleDeclaration(_)
            | Statement::TSTypeAliasDeclaration(_)
            | Statement::TSInterfaceDeclaration(_) => {}
            // Top-level-only contract: the shapes above are everything
            // this splitter hoists or strips. The module-only TS forms
            // below (`import x = require(…)`, `export =`, `declare
            // global`) and `enum` are deliberately LEFT IN THE BODY —
            // long-standing behavior locked here explicitly; hoisting
            // them is a separate decision should a real project ever
            // surface one inside a component script.
            Statement::TSEnumDeclaration(_)
            | Statement::TSGlobalDeclaration(_)
            | Statement::TSImportEqualsDeclaration(_)
            | Statement::TSExportAssignment(_)
            | Statement::TSNamespaceExportDeclaration(_) => {}
            svn_analyze::non_declaration_statement!() => {}
        }
    }

    // `export { local as exported }` (`ExportedNames.handleExportDeclaration`):
    // it counts as a `let` export exactly when `local` is a top-level
    // `let`/`var`, and is always optional.
    for (local, exported) in named_exports {
        export_type_infos.push(ExportedLocalInfo {
            is_let: top_level_lets.contains(&local),
            exported_as: (exported != local).then_some(exported),
            name: local,
            type_source: None,
            has_init: true,
            is_named_export: true,
        });
    }
    hoist_spans.extend(hoisted_type_spans(&parsed.program, hoist));
    hoist_spans.sort_unstable();

    if hoist_spans.is_empty() && strip_keyword_spans.is_empty() && drop_spans.is_empty() {
        return SplitScript {
            hoisted: String::new(),
            body: content.to_string(),
            exported_locals,
            hoisted_byte_offsets: Vec::new(),
            hoisted_stmt_spans: Vec::new(),
            export_type_infos,
        };
    }

    // Hoisted prelude: emit each hoist-span verbatim, joined by newlines.
    // Record the start byte-offset of each hoisted span IN THE ORIGINAL
    // content so callers can build a line map: each hoisted statement in
    // the overlay corresponds to the same statement in the source, and
    // diagnostics inside should map back to the right source line.
    let mut hoisted = String::new();
    let mut hoisted_byte_offsets: Vec<u32> = Vec::with_capacity(hoist_spans.len());
    let mut hoisted_stmt_spans: Vec<(usize, usize)> = Vec::with_capacity(hoist_spans.len());

    for &(start, end) in &hoist_spans {
        // Back up through same-line leading whitespace so the hoisted
        // statement keeps the source indentation. Column positions
        // then line up 1:1 between overlay and source, which matters
        // for diagnostics on hoisted `import` / `export … from`
        // statements (TS2307 module-resolution errors in particular
        // point at the specifier, and upstream svelte-check's
        // positions include the leading indent).
        let bytes = content.as_bytes();
        let mut effective_start = start;
        while effective_start > 0 {
            let b = bytes[effective_start - 1];
            if b == b' ' || b == b'\t' {
                effective_start -= 1;
            } else {
                break;
            }
        }
        hoisted_byte_offsets.push(effective_start as u32);
        let stmt_hoisted_start = hoisted.len();
        hoisted.push_str(&content[effective_start..end]);
        if !content[effective_start..end].ends_with('\n') {
            hoisted.push('\n');
        }
        hoisted_stmt_spans.push((stmt_hoisted_start, hoisted.len()));
    }

    // Body with hoisted + strip-keyword + dropped regions all blanked.
    // For strip-keyword spans we only blank the keyword prefix, not the
    // declaration — the declaration stays at its original byte position
    // in the body, with the `export ` replaced by spaces.
    let mut blank_spans: Vec<(usize, usize)> =
        Vec::with_capacity(hoist_spans.len() + strip_keyword_spans.len() + drop_spans.len());
    blank_spans.extend(hoist_spans.iter().copied());
    blank_spans.extend(strip_keyword_spans.iter().copied());
    blank_spans.extend(drop_spans.iter().copied());
    blank_spans.sort_by_key(|&(s, _)| s);

    let mut body = String::with_capacity(content.len());
    let mut cursor = 0;
    for &(start, end) in &blank_spans {
        body.push_str(&content[cursor..start]);
        // A statement written without a semicolon takes the next `;` as
        // its end, even one on a later line. That `;` often guards the
        // code after it (`export { a as b }` followed by the reactive
        // rewrite's `;() => { $: … }`), so it stays; blanking it would
        // glue that code onto the statement before the blanked one.
        let span = &content[start..end];
        let keep_semicolon = span
            .strip_suffix(';')
            .is_some_and(|rest| rest.trim_end_matches([' ', '\t']).ends_with(['\n', '\r']));
        for ch in span.chars() {
            if ch == '\n' || ch == '\r' {
                body.push(ch);
            } else if ch.is_ascii() {
                body.push(' ');
            } else {
                let byte_len = ch.len_utf8();
                for _ in 0..byte_len {
                    body.push(' ');
                }
            }
        }
        if keep_semicolon {
            body.pop();
            body.push(';');
        }
        cursor = end;
    }
    body.push_str(&content[cursor..]);

    SplitScript {
        hoisted,
        body,
        exported_locals,
        hoisted_byte_offsets,
        hoisted_stmt_spans,
        export_type_infos,
    }
}

/// Collect the local names introduced by an exported declaration.
fn collect_declaration_names(decl: &Declaration<'_>, out: &mut Vec<SmolStr>) {
    match decl {
        Declaration::VariableDeclaration(v) => {
            for d in &v.declarations {
                collect_binding_pattern_names(&d.id, out);
            }
        }
        Declaration::FunctionDeclaration(f) => {
            if let Some(id) = &f.id {
                out.push(SmolStr::from(id.name.as_str()));
            }
        }
        Declaration::ClassDeclaration(c) => {
            if let Some(id) = &c.id {
                out.push(SmolStr::from(id.name.as_str()));
            }
        }
        // `export interface`, `export type` — types, not values. Skip:
        // voiding them would fire TS2693.
        _ => {}
    }
}

pub(crate) fn collect_binding_pattern_names(pat: &BindingPattern<'_>, out: &mut Vec<SmolStr>) {
    match pat {
        BindingPattern::BindingIdentifier(id) => {
            out.push(SmolStr::from(id.name.as_str()));
        }
        BindingPattern::ObjectPattern(o) => {
            for prop in &o.properties {
                collect_binding_pattern_names(&prop.value, out);
            }
            if let Some(rest) = &o.rest {
                collect_binding_pattern_names(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(a) => {
            for el in a.elements.iter().flatten() {
                collect_binding_pattern_names(el, out);
            }
            if let Some(rest) = &a.rest {
                collect_binding_pattern_names(&rest.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(a) => {
            collect_binding_pattern_names(&a.left, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_imports_or_exports_passes_through() {
        let s = split_imports("let x = 1;", ScriptLang::Js, &HoistContext::default());
        assert_eq!(s.hoisted, "");
        assert_eq!(s.body, "let x = 1;");
    }

    #[test]
    fn single_import_is_hoisted() {
        let src = "import { writable } from 'svelte/store';\nlet x = 1;";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(
            s.hoisted
                .contains("import { writable } from 'svelte/store';")
        );
        assert!(s.body.contains("let x = 1;"));
        assert!(!s.body.contains("import"));
    }

    #[test]
    fn multiple_imports_all_hoisted() {
        let src = "\
import a from 'a';
import b from 'b';
let x = 1;
";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(s.hoisted.contains("import a from 'a';"));
        assert!(s.hoisted.contains("import b from 'b';"));
        assert!(s.body.contains("let x = 1;"));
    }

    #[test]
    fn type_only_imports_hoisted() {
        let src = "import type { Foo } from './foo';\nlet x: Foo = bar;";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(s.hoisted.contains("import type { Foo }"));
    }

    #[test]
    fn export_const_keyword_is_stripped_keeping_declaration_in_body() {
        // The declaration body is what we care about for type-checking.
        // The `export ` prefix is blanked but `const PI = 3.14;` stays
        // at its original position in the body.
        let src = "let x = 1;\nexport const PI = 3.14;";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(
            !s.hoisted.contains("export"),
            "should not hoist:\n{}",
            s.hoisted
        );
        assert!(
            !s.body.contains("export"),
            "should be blanked from body:\n{}",
            s.body
        );
        assert!(
            s.body.contains("const PI = 3.14;"),
            "declaration must survive:\n{}",
            s.body
        );
    }

    #[test]
    fn export_function_keyword_is_stripped() {
        // Svelte 5 component-level method export. Keyword stripped so
        // the function body's references (which may use other locals)
        // stay in scope.
        let src = "let x = $state(0);\nexport function foo() { return x; }";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(!s.hoisted.contains("export"));
        assert!(
            s.body.contains("function foo()"),
            "function declaration kept:\n{}",
            s.body
        );
        assert!(s.body.contains("let x = $state(0);"));
    }

    #[test]
    fn re_export_list_without_source_is_dropped_not_hoisted() {
        // `export { a, b }` (no `from` clause) re-exports local names.
        // Hoisting it to module level would fire TS2304/TS2552 because
        // `a` and `b` live inside $$render. We drop it entirely; the
        // declarations themselves stay intact in the body.
        let src = "let a = 1;\nlet b = 2;\nexport { a, b };";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(
            !s.hoisted.contains("export { a, b }"),
            "re-export without source should NOT be hoisted:\n{}",
            s.hoisted
        );
        assert!(
            !s.body.contains("export { a, b }"),
            "should be blanked from body"
        );
        assert!(s.body.contains("let a = 1;"));
        assert!(s.body.contains("let b = 2;"));
    }

    #[test]
    fn renamed_re_export_without_source_is_dropped() {
        let src = "let a = 1;\nexport { a as renamed };";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(!s.hoisted.contains("export"));
        assert!(!s.body.contains("export"));
    }

    #[test]
    fn re_export_with_source_is_hoisted() {
        // `export { x } from 'mod'` doesn't reference local names — it's a
        // pure module-to-module re-export. Safe to hoist.
        let src = "export { foo } from './other';";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(s.hoisted.contains("export { foo } from './other';"));
        assert!(!s.body.contains("export"));
    }

    #[test]
    fn export_default_stays_in_the_body() {
        let src = "let x = 1;\nexport default x;";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(!s.hoisted.contains("export default"));
        assert_eq!(s.body, src);
    }

    #[test]
    fn every_leading_comment_moves_with_its_import() {
        let src = "let x = 1;\n// note\n/* @ts-ignore */ import a from 'a';\nlet y = 2; // trailing\nimport b from 'b';";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(
            s.hoisted
                .contains("// note\n/* @ts-ignore */ import a from 'a';"),
            "{}",
            s.hoisted
        );
        // A comment after the previous statement on its line leads the
        // next statement too.
        assert!(
            s.hoisted.contains("// trailing\nimport b from 'b';"),
            "{}",
            s.hoisted
        );
        assert!(!s.body.contains("note"));
    }

    #[test]
    fn export_star_re_export_is_hoisted() {
        let src = "export * from './other';";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(s.hoisted.contains("export * from './other';"));
        assert!(!s.body.contains("export"));
    }

    /// A namespace is illegal inside a function, and the instance
    /// script becomes one — so TS1235 is the right answer for writing
    /// one there. Hoisting the namespace out would suppress a
    /// diagnostic the user should see, which is why upstream leaves it
    /// in place (`HoistableInterfaces.analyzeInstanceScriptNode`) and
    /// its `ts-runes-hoistable-props-false-12.v5` fixture shows the
    /// namespace still inside `$$render()`. We match that.
    #[test]
    fn instance_script_namespace_stays_in_the_body() {
        let src = "let x = 1;\nnamespace Foo { export type Bar = number; }";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(
            !s.hoisted.contains("namespace Foo"),
            "namespace must not be hoisted:\n{}",
            s.hoisted
        );
        assert!(
            s.body.contains("namespace Foo"),
            "namespace must survive in the body:\n{}",
            s.body
        );
        assert!(s.body.contains("let x = 1;"));
    }

    /// The other half of upstream's rule: a type that depends on an
    /// instance-script namespace has to stay beside it, since hoisting
    /// it would move it to a scope where the namespace isn't visible.
    #[test]
    fn types_depending_on_a_namespace_stay_in_the_body() {
        let src = "namespace A { export type Abc = number; }\ninterface Props { foo: A.Abc }";
        let s = split_imports(src, ScriptLang::Ts, &typed_props("Props"));
        assert!(
            !s.hoisted.contains("interface Props"),
            "dependent interface must not be hoisted:\n{}",
            s.hoisted
        );
        assert!(s.body.contains("interface Props"));
    }

    /// The dotted form nests declarations rather than holding a block,
    /// so a reader looking for the namespace's contents finds something
    /// different in shape. Nothing here depends on the contents, and
    /// the outer name is a plain identifier either way.
    #[test]
    fn dotted_and_nested_namespaces_behave_like_the_flat_form() {
        for src in [
            "const a = 1;\nnamespace Outer.Inner { export type C = typeof a; }",
            "const a = 1;\nnamespace Outer { export namespace Inner { export type C = typeof a; } }",
        ] {
            let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
            assert!(
                !s.hoisted.contains("namespace"),
                "must not be hoisted:\n{}",
                s.hoisted
            );
            assert!(s.body.contains("namespace Outer"));
        }
    }

    /// `declare module 'foo'` is a string-named external module, not a
    /// namespace. It stays in the body too, and introduces no name a
    /// local type could depend on.
    #[test]
    fn external_module_declaration_stays_in_the_body() {
        let src = "declare module 'foo' { export const x: number; }\nlet y = 1;";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(!s.hoisted.contains("declare module"));
        assert!(s.body.contains("declare module 'foo'"));
    }

    #[test]
    fn body_offsets_preserved() {
        let src = "import a from 'a';\nlet x = 1;\nexport const y = 2;\nlet z = 3;";
        let original_let_z = src.find("let z").unwrap();
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        let new_let_z = s.body.find("let z").unwrap();
        assert_eq!(new_let_z, original_let_z);
    }

    #[test]
    fn newlines_preserved_inside_blanked_regions() {
        let src = "\
import {
    a,
    b,
} from 'mod';
let x = 1;
";
        let original_x_line = src.lines().position(|l| l.contains("let x")).unwrap();
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        let new_x_line = s.body.lines().position(|l| l.contains("let x")).unwrap();
        assert_eq!(new_x_line, original_x_line);
    }

    #[test]
    fn blanking_keeps_a_semicolon_borrowed_from_a_later_line() {
        // Without its own `;`, the export list ends at the `;` that starts
        // the next line. That `;` separates `let a = ''` from the arrow.
        let src = "let a = ''\nexport { a as b }\n;() => {};\n";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert_eq!(s.body, "let a = ''\n                 \n;() => {};\n");
        assert_eq!(s.body.len(), src.len());

        // A statement's own `;` is still blanked.
        let src = "let a = ''\nexport { a as b };\n() => {};\n";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert_eq!(s.body, "let a = ''\n                  \n() => {};\n");
    }

    #[test]
    fn malformed_script_falls_back_to_passthrough() {
        let src = "import {{{ unbalanced";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        let total = format!("{}{}", s.hoisted, s.body);
        assert!(total.contains("import"));
    }

    #[test]
    fn export_type_specifier_not_void_emitted() {
        // `export { type Bar }` — type-only specifier. The declaration
        // list gets dropped; the type name must NOT be recorded in
        // exported_locals because emit would wrap it in `void Bar;`
        // which fires TS2693 on a type name.
        let src = "type Bar = string;\nexport { type Bar };";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(
            !s.exported_locals.iter().any(|n| n == "Bar"),
            "type-only specifier must not be voided:\n{:?}",
            s.exported_locals
        );
    }

    #[test]
    fn export_type_decl_specifier_list_not_void_emitted() {
        // `export type { Bar }` — whole declaration marked type-only.
        // Same rule: don't void the name.
        let src = "type Bar = string;\nexport type { Bar };";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(
            !s.exported_locals.iter().any(|n| n == "Bar"),
            "whole-decl type export must not be voided:\n{:?}",
            s.exported_locals
        );
    }

    #[test]
    fn mixed_value_and_type_specifier_only_value_voided() {
        // `export { Foo, type Bar }` — Foo is a runtime name (goes to
        // exported_locals for void-emission), Bar is a type (skipped).
        let src = "let Foo = 1;\ntype Bar = string;\nexport { Foo, type Bar };";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(
            s.exported_locals.iter().any(|n| n == "Foo"),
            "value specifier missing:\n{:?}",
            s.exported_locals
        );
        assert!(
            !s.exported_locals.iter().any(|n| n == "Bar"),
            "type specifier must not be voided:\n{:?}",
            s.exported_locals
        );
    }

    fn typed_props(name: &str) -> HoistContext {
        HoistContext {
            props_type:
                crate::svelte2tsx_nodes::hoistable_interfaces::PropsTypeShape::from_type_text(name),
            ..HoistContext::default()
        }
    }

    /// Without a typed `$props()` nothing moves: an exported type stays
    /// in the render function exactly as written, `export` included.
    #[test]
    fn exported_types_stay_in_the_body_without_typed_props() {
        let src =
            "let x = 1;\nexport type Foo = string | number;\nexport interface Bar { n: number; }";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert_eq!(s.hoisted, "");
        assert!(s.body.contains("export type Foo = string | number;"));
        assert!(s.body.contains("export interface Bar { n: number; }"));
    }

    /// With a hoistable props type, every hoistable declaration moves,
    /// exported or not, whether or not the props type uses it.
    #[test]
    fn typed_props_move_every_hoistable_type() {
        let src = "export type Foo = string;\ntype Unused = number;\ninterface Props { f: Foo }";
        let s = split_imports(src, ScriptLang::Ts, &typed_props("Props"));
        assert!(
            s.hoisted.contains("export type Foo = string;"),
            "{}",
            s.hoisted
        );
        assert!(s.hoisted.contains("type Unused = number;"), "{}", s.hoisted);
        assert!(s.hoisted.contains("interface Props"), "{}", s.hoisted);
        assert!(!s.body.contains("type"), "{}", s.body);
    }

    /// A `typeof` read of an instance-script value pins the type (and
    /// everything that names it) inside the function; when that is the
    /// props type, nothing moves at all.
    #[test]
    fn typeof_an_instance_value_blocks_hoisting() {
        let src = "const arr = [1] as const;\ntype X = (typeof arr)[number];\ntype Y = string;\ninterface Props { x: X }";
        let s = split_imports(src, ScriptLang::Ts, &typed_props("Props"));
        assert_eq!(s.hoisted, "");
        // A property key that happens to share a local's name is not a
        // dependency.
        let src = "const x = 1;\ninterface Props { x: number }";
        let s = split_imports(src, ScriptLang::Ts, &typed_props("Props"));
        assert!(s.hoisted.contains("interface Props"), "{}", s.hoisted);
    }

    /// Imports are module-scope values, so `typeof` of one is fine —
    /// unless the component subscribes to it as a store.
    #[test]
    fn typeof_an_import_hoists_unless_it_is_a_store() {
        let src = "import { foo } from 'mod';\ntype X = typeof foo;\ninterface Props { x: X }";
        let s = split_imports(src, ScriptLang::Ts, &typed_props("Props"));
        assert!(s.hoisted.contains("type X = typeof foo;"), "{}", s.hoisted);
        let ctx = HoistContext {
            accessed_stores: vec![SmolStr::new("foo")],
            ..typed_props("Props")
        };
        let s = split_imports(src, ScriptLang::Ts, &ctx);
        assert!(!s.hoisted.contains("type X"), "{}", s.hoisted);
    }

    /// A name the module script declares as a type is shadowed inside
    /// the function, and a script generic only exists there.
    #[test]
    fn module_types_and_generics_stay_in_the_body() {
        let src = "interface Props { a: number }";
        let ctx = HoistContext {
            module_script: Some("import type { Props } from './types';".to_string()),
            ..typed_props("Props")
        };
        let s = split_imports(src, ScriptLang::Ts, &ctx);
        assert_eq!(s.hoisted, "");

        let src = "type Props = { item: T };";
        let ctx = HoistContext {
            generic_names: vec![SmolStr::new("T")],
            ..typed_props("Props")
        };
        let s = split_imports(src, ScriptLang::Ts, &ctx);
        assert_eq!(s.hoisted, "");
    }

    /// An inline props type moves what it names when it can itself.
    #[test]
    fn inline_props_type_moves_its_dependencies() {
        let src = "type A = string;\nlet { a }: { a: A } = $props();";
        let s = split_imports(src, ScriptLang::Ts, &typed_props("{ a: A }"));
        assert!(s.hoisted.contains("type A = string;"), "{}", s.hoisted);
    }

    /// A `$$Generic<Name>` constraint moves `Name` regardless of props.
    #[test]
    fn generic_constraint_type_moves() {
        let src = "interface Item { id: number }\nexport let item: T;";
        let ctx = HoistContext {
            generic_names: vec![SmolStr::new("T")],
            generic_constraints: vec!["Item".to_string()],
            ..HoistContext::default()
        };
        let s = split_imports(src, ScriptLang::Ts, &ctx);
        assert!(
            s.hoisted.contains("interface Item { id: number }"),
            "{}",
            s.hoisted
        );
    }

    #[test]
    fn import_and_export_in_same_script() {
        // Import gets hoisted; bare re-export gets dropped (its name lives
        // inside $$render).
        let src = "\
import { writable } from 'svelte/store';
let count = writable(0);
export { count };
";
        let s = split_imports(src, ScriptLang::Ts, &HoistContext::default());
        assert!(s.hoisted.contains("import { writable }"));
        assert!(!s.hoisted.contains("export { count }"));
        assert!(s.body.contains("let count = writable(0);"));
    }
}
