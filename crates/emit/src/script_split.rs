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
//! - **`export default x`** — TS1232
//! - **`export * from 'mod'`** — TS1232
//!
//! All are hoisted to a module-level prelude. The original spans inside
//! the script body are blanked with whitespace of the same byte length so
//! line/column positions inside the body stay aligned for source-map
//! mapping.

use std::collections::HashSet;

use oxc_allocator::Allocator;
use oxc_ast::ast::{BindingPatternKind, Declaration, ImportOrExportKind, Statement};
use oxc_span::GetSpan;
use smol_str::SmolStr;
use svn_parser::{ScriptLang, parse_script_body};

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
}

/// Split out every module-level statement (imports, exports of all
/// shapes) from a script body.
///
/// Re-parses the body once with oxc. If parsing panics on malformed user
/// code, the content is passed through unchanged.
///
/// `has_generics` toggles the hoist behavior for bare `type Foo = ...`
/// and `interface Foo { ... }` declarations. When the component
/// declares generic type parameters via `<script generics="T extends
/// ...">`, those types live only inside the `$$render<T>(...)`
/// function. Hoisting a `type Props = { item: T; }` declaration to the
/// overlay module scope would surface `T` in a context where the
/// render function's generic parameters don't exist, producing
/// "Cannot find name 'T'". Leaving the declaration in the body keeps
/// `T` in scope but forfeits the ability to type the default export as
/// `Component<Props>` (see the caller for that fallback).
///
/// `export type Foo = ...` / `export interface Foo { ... }` are always
/// hoisted regardless — hoisting them is the only way to make them
/// available to consumers via `import type { Foo } from './X.svelte'`,
/// and those types are user-facing surface so they're unlikely to
/// reference private render-scope generics.
pub fn split_imports(content: &str, _lang: ScriptLang, has_generics: bool) -> SplitScript {
    // Fast path: none of the hoistable shapes appear as substrings →
    // skip the parse. `type ` and `interface ` catch TS type/interface
    // declarations which we now hoist too (so the module-level default
    // export can reference a user-declared `Props` type). `namespace`/
    // `module` cover the `TSModuleDeclaration` case.
    if !content.contains("import")
        && !content.contains("export")
        && !content.contains("interface ")
        && !content.contains("type ")
        && !content.contains("namespace ")
        && !content.contains("module ")
    {
        return SplitScript {
            hoisted: String::new(),
            body: content.to_string(),
            exported_locals: Vec::new(),
            hoisted_byte_offsets: Vec::new(),
        };
    }

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
        };
    }

    // Names declared at the top level of the body — const/let/var,
    // function, class. Populated as we iterate `parsed.program.body`.
    // Imports aren't included: imports that get hoisted to module top
    // level are already visible there, so type aliases referencing them
    // resolve without needing a `declare` stub.
    //
    // Used at the end of this function to decide which names need a
    // module-level `declare const <name>: any;` stub so that hoisted
    // type aliases / interfaces referring to them resolve. Example: the
    // user writes
    //   ```
    //   const standaloneChartTypes = ['a', 'b'] as const
    //   type StandaloneChartType = (typeof standaloneChartTypes)[number]
    //   ```
    // We hoist the `type` but keep the `const` in the `$$render` body.
    // Without a stub, the hoisted `type` fires "Cannot find name
    // 'standaloneChartTypes'" at module scope.
    let mut body_decl_names: Vec<SmolStr> = Vec::new();
    // Spans we hoist verbatim to module top level. For statements that
    // are pure module-shape (no references to body locals): imports,
    // `export { x } from 'mod'`, `export * from 'mod'`.
    let mut hoist_spans: Vec<(usize, usize)> = Vec::new();
    // Spans where we strip just the `export ` prefix and let the inner
    // declaration stay in the body. For `export const/let/var/function/class`
    // — the declaration body might reference locals (e.g. `export function
    // getA() { return a; }` where `a` is a local), so hoisting would
    // break those references. Stripping the keyword keeps everything in
    // scope; the consumer-facing export goes away (consumers can't
    // `import { foo } from './X.svelte'` for these names) but the body
    // type-checks cleanly.
    let mut strip_keyword_spans: Vec<(usize, usize)> = Vec::new();
    // Spans we drop entirely (blank in body, don't add to hoisted prelude).
    // For `export { x, y }` (no `from`) re-exports of local names, and
    // `export default x` where x is a name (we can't easily distinguish
    // expression-vs-name without more parsing — drop is safer).
    let mut drop_spans: Vec<(usize, usize)> = Vec::new();
    let mut exported_locals: Vec<SmolStr> = Vec::new();

    for stmt in &parsed.program.body {
        match stmt {
            Statement::ImportDeclaration(decl) => {
                hoist_spans.push((decl.span.start as usize, decl.span.end as usize));
            }
            Statement::VariableDeclaration(decl) => {
                // Body-level `const/let/var` — stays in body. Record its
                // names for the `declare const` stub pass.
                for d in &decl.declarations {
                    collect_binding_pattern_names(&d.id.kind, &mut body_decl_names);
                }
            }
            Statement::FunctionDeclaration(decl) => {
                if let Some(id) = &decl.id {
                    body_decl_names.push(SmolStr::from(id.name.as_str()));
                }
            }
            Statement::ClassDeclaration(decl) => {
                if let Some(id) = &decl.id {
                    body_decl_names.push(SmolStr::from(id.name.as_str()));
                }
            }
            Statement::ExportNamedDeclaration(decl) => {
                let span = (decl.span.start as usize, decl.span.end as usize);
                if let Some(d) = &decl.declaration {
                    // `export type Foo = ...` / `export interface Foo { ... }` —
                    // pure type-namespace declarations. Hoist the whole
                    // statement (including the `export` keyword) so the
                    // overlay's module top level carries the export and
                    // consumers writing `import type { Foo } from './X.svelte'`
                    // resolve. Without this branch we'd fall through to the
                    // strip-keyword path below, which would blank the
                    // `export ` prefix and leave `type Foo = ...` in the
                    // function body — invisible to consumers and never
                    // re-exported by the overlay.
                    if matches!(
                        d,
                        Declaration::TSTypeAliasDeclaration(_)
                            | Declaration::TSInterfaceDeclaration(_)
                    ) {
                        hoist_spans.push(span);
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
                    // Record body-level names too: the declaration stays
                    // in the body after the `export ` prefix is stripped,
                    // so a hoisted `type X = typeof name` needs a
                    // `declare` stub just like for a non-exported
                    // body-level const.
                    collect_declaration_names(d, &mut body_decl_names);
                } else if decl.source.is_some() {
                    // `export { x } from 'mod'` — pure module re-export,
                    // no local name references. Hoist.
                    hoist_spans.push(span);
                } else {
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
                        exported_locals.push(SmolStr::from(spec.local.name().as_str()));
                    }
                }
            }
            Statement::ExportDefaultDeclaration(decl) => {
                // `export default <expr>` — drop. Expressions may reference
                // locals; we don't try to disambiguate. The default export
                // surface goes away but the body keeps type-checking.
                drop_spans.push((decl.span.start as usize, decl.span.end as usize));
            }
            Statement::ExportAllDeclaration(decl) => {
                hoist_spans.push((decl.span.start as usize, decl.span.end as usize));
            }
            // TypeScript `namespace Foo { ... }` (and the equivalent
            // `module Foo { ... }`). Allowed only at the module level
            // (TS1235 inside a function); hoist verbatim.
            Statement::TSModuleDeclaration(decl) => {
                hoist_spans.push((decl.span.start as usize, decl.span.end as usize));
            }
            // `type Foo = ...` and `interface Foo { ... }` — hoist so
            // the emitted `declare const __svn_component_default:
            // Component<Foo>` at module level can reference them.
            // Scripts typically declare `interface Props { ... }` right
            // before `let { ... }: Props = $props()`; without the
            // hoist, the overlay's typed default export reads "Cannot
            // find name 'Props'" at module top.
            //
            // Exception: when the component has `<script
            // generics="T...">`, the script's type aliases likely
            // reference those type parameters. Hoisting a
            // `type Props = { item: T }` out of `$$render<T>()`
            // into the module scope leaves `T` unbound at the use
            // site. Keep those in the body; the caller then types the
            // default export as `any` (giving up
            // `ComponentProps<typeof X>` flow for generic components,
            // which is rare enough to accept).
            Statement::TSTypeAliasDeclaration(decl) => {
                if !has_generics {
                    hoist_spans.push((decl.span.start as usize, decl.span.end as usize));
                }
            }
            Statement::TSInterfaceDeclaration(decl) => {
                if !has_generics {
                    hoist_spans.push((decl.span.start as usize, decl.span.end as usize));
                }
            }
            _ => {}
        }
    }

    if hoist_spans.is_empty() && strip_keyword_spans.is_empty() && drop_spans.is_empty() {
        return SplitScript {
            hoisted: String::new(),
            body: content.to_string(),
            exported_locals,
            hoisted_byte_offsets: Vec::new(),
        };
    }

    // Hoisted prelude: emit each hoist-span verbatim, joined by newlines.
    // Record the start byte-offset of each hoisted span IN THE ORIGINAL
    // content so callers can build a line map: each hoisted statement in
    // the overlay corresponds to the same statement in the source, and
    // diagnostics inside should map back to the right source line.
    let mut hoisted = String::new();
    let mut hoisted_byte_offsets: Vec<u32> = Vec::with_capacity(hoist_spans.len());

    // `declare const` stubs for body-level names referenced from inside
    // hoisted type aliases / interfaces. The stubs go FIRST (ahead of
    // the real hoisted statements) so a subsequent `type X = typeof
    // name` resolves its reference at module scope. We only scan the
    // hoisted spans that correspond to type aliases / interfaces /
    // namespaces — value-shape hoists (imports, `export { x } from`,
    // `export * from`) can't reference body-local names anyway.
    //
    // No source-line mapping for these — they're synthetic and tsgo is
    // expected to never fire a diagnostic on them (`any`-typed
    // placeholders).
    let body_names_set: HashSet<SmolStr> = body_decl_names.iter().cloned().collect();
    let referenced: HashSet<SmolStr> = if body_names_set.is_empty() {
        HashSet::new()
    } else {
        // Scan every HOISTED span (i.e. what we're actually about to
        // emit to module scope) and find body-level name references.
        // Only type-alias / interface / namespace / `export type`
        // hoists can reference body locals in a TS sense; plain
        // `import`/`export … from 'mod'` spans live in their own
        // module-level namespace. Over-scanning is harmless — the
        // intersection with `body_names_set` filters non-matches.
        //
        // Importantly, the pending-type-span pass above already
        // REMOVED body-referencing type aliases from `hoist_spans`
        // (they stay in body to preserve type identity). So this
        // scan only finds names from types that the decision pass
        // kept hoistable — ones where we're confident a `declare
        // const` stub won't degrade a `typeof`/`keyof` into `any`.
        let mut scan_text = String::new();
        for &(start, end) in &hoist_spans {
            scan_text.push_str(&content[start..end]);
            scan_text.push('\n');
        }
        collect_ident_refs(&scan_text)
            .into_iter()
            .filter(|n| body_names_set.contains(n))
            .collect()
    };
    // Emit stubs in original declaration order, to keep diffs stable.
    //
    // The stub's type is `{ [key: string]: any }` rather than plain
    // `any` so downstream `typeof <name>` / `keyof typeof <name>`
    // patterns retain enough structure for TS to reason about. On the
    // stub-as-`any` path, `keyof typeof <name>` widens to
    // `string | number | symbol` and the user's `<name>[stringKey]`
    // then fires "Type 'symbol' cannot be used as an index type".
    // Using an index signature preserves `keyof <stub> = string` and
    // still yields `any` on subscript, which is what the user
    // actually wants when we can't see the real type.
    let mut stub_seen: HashSet<SmolStr> = HashSet::new();
    for name in &body_decl_names {
        if referenced.contains(name) && stub_seen.insert(name.clone()) {
            hoisted.push_str("declare const ");
            hoisted.push_str(name);
            hoisted.push_str(": { [key: string]: any } & ((...args: any[]) => any);\n");
        }
    }

    for &(start, end) in &hoist_spans {
        hoisted_byte_offsets.push(start as u32);
        hoisted.push_str(&content[start..end]);
        if !content[start..end].ends_with('\n') {
            hoisted.push('\n');
        }
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
        for ch in content[start..end].chars() {
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
        cursor = end;
    }
    body.push_str(&content[cursor..]);

    SplitScript {
        hoisted,
        body,
        exported_locals,
        hoisted_byte_offsets,
    }
}

/// Collect the local names introduced by an exported declaration.
fn collect_declaration_names(decl: &Declaration<'_>, out: &mut Vec<SmolStr>) {
    match decl {
        Declaration::VariableDeclaration(v) => {
            for d in &v.declarations {
                collect_binding_pattern_names(&d.id.kind, out);
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

/// Byte-scan a JS/TS source slice for identifier references.
///
/// Returns every identifier that appears NOT after a `.` or `?.` (so
/// `obj.prop` yields `obj`, not `prop`). Skips string literals,
/// template-literal text (but recurses into `${...}` substitutions),
/// and line/block comments. A keyword/built-in list is filtered out
/// so `typeof`, `keyof`, etc. don't leak into the result.
///
/// The scanner is intentionally lenient — false positives (e.g. a
/// property key in an object literal) are acceptable because the
/// caller intersects with a known set of body-declared names.
fn collect_ident_refs(text: &str) -> Vec<SmolStr> {
    let mut seen: HashSet<SmolStr> = HashSet::new();
    let mut out: Vec<SmolStr> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
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
                    let inner = &text[inner_start..i];
                    for sub in collect_ident_refs(inner) {
                        if seen.insert(sub.clone()) {
                            out.push(sub);
                        }
                    }
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
            let name = &text[start..i];
            if !after_dot && !is_ref_scan_keyword(name) {
                let s = SmolStr::from(name);
                if seen.insert(s.clone()) {
                    out.push(s);
                }
            }
            after_dot = false;
            continue;
        }
        // Member access — suppress next identifier.
        if b == b'.' {
            after_dot = true;
            i += 1;
            continue;
        }
        if !b.is_ascii_whitespace() {
            after_dot = false;
        }
        i += 1;
    }

    out
}

/// Keywords/built-ins that appear frequently in TS type annotations
/// and should never be treated as a reference.
fn is_ref_scan_keyword(s: &str) -> bool {
    matches!(
        s,
        "true"
            | "false"
            | "null"
            | "undefined"
            | "this"
            | "void"
            | "typeof"
            | "keyof"
            | "infer"
            | "extends"
            | "in"
            | "of"
            | "as"
            | "is"
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
            | "super"
            | "import"
            | "export"
            | "from"
            | "satisfies"
            | "readonly"
            | "type"
            | "interface"
            | "namespace"
            | "module"
            | "declare"
            | "public"
            | "private"
            | "protected"
            | "new"
            | "instanceof"
            | "any"
            | "unknown"
            | "never"
            | "number"
            | "string"
            | "boolean"
            | "symbol"
            | "object"
            | "bigint"
            | "Array"
            | "ReadonlyArray"
            | "Record"
            | "Partial"
            | "Required"
            | "Pick"
            | "Omit"
            | "Exclude"
            | "Extract"
            | "NonNullable"
            | "Parameters"
            | "ReturnType"
            | "InstanceType"
            | "Awaited"
            | "Promise"
    )
}

fn collect_binding_pattern_names(pat: &BindingPatternKind<'_>, out: &mut Vec<SmolStr>) {
    match pat {
        BindingPatternKind::BindingIdentifier(id) => {
            out.push(SmolStr::from(id.name.as_str()));
        }
        BindingPatternKind::ObjectPattern(o) => {
            for prop in &o.properties {
                collect_binding_pattern_names(&prop.value.kind, out);
            }
            if let Some(rest) = &o.rest {
                collect_binding_pattern_names(&rest.argument.kind, out);
            }
        }
        BindingPatternKind::ArrayPattern(a) => {
            for el in a.elements.iter().flatten() {
                collect_binding_pattern_names(&el.kind, out);
            }
            if let Some(rest) = &a.rest {
                collect_binding_pattern_names(&rest.argument.kind, out);
            }
        }
        BindingPatternKind::AssignmentPattern(a) => {
            collect_binding_pattern_names(&a.left.kind, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_imports_or_exports_passes_through() {
        let s = split_imports("let x = 1;", ScriptLang::Js, false);
        assert_eq!(s.hoisted, "");
        assert_eq!(s.body, "let x = 1;");
    }

    #[test]
    fn single_import_is_hoisted() {
        let src = "import { writable } from 'svelte/store';\nlet x = 1;";
        let s = split_imports(src, ScriptLang::Ts, false);
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
        let s = split_imports(src, ScriptLang::Ts, false);
        assert!(s.hoisted.contains("import a from 'a';"));
        assert!(s.hoisted.contains("import b from 'b';"));
        assert!(s.body.contains("let x = 1;"));
    }

    #[test]
    fn type_only_imports_hoisted() {
        let src = "import type { Foo } from './foo';\nlet x: Foo = bar;";
        let s = split_imports(src, ScriptLang::Ts, false);
        assert!(s.hoisted.contains("import type { Foo }"));
    }

    #[test]
    fn export_const_keyword_is_stripped_keeping_declaration_in_body() {
        // The declaration body is what we care about for type-checking.
        // The `export ` prefix is blanked but `const PI = 3.14;` stays
        // at its original position in the body.
        let src = "let x = 1;\nexport const PI = 3.14;";
        let s = split_imports(src, ScriptLang::Ts, false);
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
        let s = split_imports(src, ScriptLang::Ts, false);
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
        let s = split_imports(src, ScriptLang::Ts, false);
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
        let s = split_imports(src, ScriptLang::Ts, false);
        assert!(!s.hoisted.contains("export"));
        assert!(!s.body.contains("export"));
    }

    #[test]
    fn re_export_with_source_is_hoisted() {
        // `export { x } from 'mod'` doesn't reference local names — it's a
        // pure module-to-module re-export. Safe to hoist.
        let src = "export { foo } from './other';";
        let s = split_imports(src, ScriptLang::Ts, false);
        assert!(s.hoisted.contains("export { foo } from './other';"));
        assert!(!s.body.contains("export"));
    }

    #[test]
    fn export_default_is_dropped() {
        // `export default x` could reference a local; we don't try to
        // disambiguate. Drop is safer than hoisting. Consumer-side
        // default-export surface goes away but body type-checks.
        let src = "let x = 1;\nexport default x;";
        let s = split_imports(src, ScriptLang::Ts, false);
        assert!(!s.hoisted.contains("export default"));
        assert!(!s.body.contains("export default"));
        assert!(s.body.contains("let x = 1;"));
    }

    #[test]
    fn export_star_re_export_is_hoisted() {
        let src = "export * from './other';";
        let s = split_imports(src, ScriptLang::Ts, false);
        assert!(s.hoisted.contains("export * from './other';"));
        assert!(!s.body.contains("export"));
    }

    #[test]
    fn typescript_namespace_is_hoisted() {
        // `namespace Foo { ... }` is illegal inside a function (TS1235);
        // must be lifted to module level.
        let src = "let x = 1;\nnamespace Foo { export type Bar = number; }";
        let s = split_imports(src, ScriptLang::Ts, false);
        assert!(
            s.hoisted.contains("namespace Foo"),
            "namespace must be hoisted:\n{}",
            s.hoisted
        );
        assert!(
            !s.body.contains("namespace"),
            "blanked from body:\n{}",
            s.body
        );
        assert!(s.body.contains("let x = 1;"));
    }

    #[test]
    fn body_offsets_preserved() {
        let src = "import a from 'a';\nlet x = 1;\nexport const y = 2;\nlet z = 3;";
        let original_let_z = src.find("let z").unwrap();
        let s = split_imports(src, ScriptLang::Ts, false);
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
        let s = split_imports(src, ScriptLang::Ts, false);
        let new_x_line = s.body.lines().position(|l| l.contains("let x")).unwrap();
        assert_eq!(new_x_line, original_x_line);
    }

    #[test]
    fn malformed_script_falls_back_to_passthrough() {
        let src = "import {{{ unbalanced";
        let s = split_imports(src, ScriptLang::Ts, false);
        let total = format!("{}{}", s.hoisted, s.body);
        assert!(total.contains("import"));
    }

    #[test]
    fn export_type_alias_is_hoisted_with_export_keyword() {
        // `export type Foo = ...` is a pure type-namespace declaration
        // that's legal at module top level. Hoist the whole statement so
        // consumers writing `import type { Foo } from './X.svelte'`
        // resolve. Stripping just the `export ` would leave the type in
        // the function body, invisible to consumers.
        let src = "let x = 1;\nexport type Foo = string | number;";
        let s = split_imports(src, ScriptLang::Ts, false);
        assert!(
            s.hoisted.contains("export type Foo = string | number;"),
            "export type must be hoisted verbatim:\n{}",
            s.hoisted
        );
        assert!(
            !s.body.contains("type Foo"),
            "declaration must be removed from body:\n{}",
            s.body
        );
    }

    #[test]
    fn export_interface_is_hoisted_with_export_keyword() {
        let src = "let x = 1;\nexport interface Foo { n: number; }";
        let s = split_imports(src, ScriptLang::Ts, false);
        assert!(
            s.hoisted.contains("export interface Foo { n: number; }"),
            "export interface must be hoisted verbatim:\n{}",
            s.hoisted
        );
        assert!(!s.body.contains("interface Foo"));
    }

    #[test]
    fn export_type_specifier_not_void_emitted() {
        // `export { type Bar }` — type-only specifier. The declaration
        // list gets dropped; the type name must NOT be recorded in
        // exported_locals because emit would wrap it in `void Bar;`
        // which fires TS2693 on a type name.
        let src = "type Bar = string;\nexport { type Bar };";
        let s = split_imports(src, ScriptLang::Ts, false);
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
        let s = split_imports(src, ScriptLang::Ts, false);
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
        let s = split_imports(src, ScriptLang::Ts, false);
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

    #[test]
    fn hoisted_type_referencing_body_const_emits_declare_stub() {
        // `type X = (typeof arr)[number]` is hoisted to module scope, but
        // `arr` is a body-level const and stays in $$render. Without a
        // module-level stub the hoisted type fires "Cannot find name
        // 'arr'". Using an index-signature + callable intersection (not
        // plain `any`) preserves `keyof typeof arr = string` so
        // downstream user code like `arr[key]` doesn't fire "symbol
        // cannot be used as an index type".
        let src = "const arr = [1, 2, 3] as const;\ntype X = (typeof arr)[number];";
        let s = split_imports(src, ScriptLang::Ts, false);
        assert!(
            s.hoisted.contains("declare const arr:"),
            "expected declare stub for body-local const:\n{}",
            s.hoisted
        );
        assert!(
            s.hoisted.contains("type X ="),
            "type alias should still be hoisted:\n{}",
            s.hoisted
        );
        // Stub must come before the type alias so the reference resolves.
        let stub_pos = s.hoisted.find("declare const arr").unwrap();
        let type_pos = s.hoisted.find("type X").unwrap();
        assert!(stub_pos < type_pos);
    }

    #[test]
    fn hoisted_type_referencing_body_function_emits_declare_stub() {
        // `typeof foo` inside a hoisted interface resolves once we emit
        // a module-level stub.
        let src = "async function foo() { return 1; }\ninterface Props { cb: typeof foo }\n";
        let s = split_imports(src, ScriptLang::Ts, false);
        assert!(
            s.hoisted.contains("declare const foo:"),
            "expected declare stub for body-local function:\n{}",
            s.hoisted
        );
        assert!(s.hoisted.contains("interface Props"));
    }

    #[test]
    fn no_declare_stub_for_names_not_referenced_by_hoisted_types() {
        // `unused` is a body-local const but no hoisted type references
        // it, so we don't emit a stub (avoids clutter + prevents
        // collisions with names that happen to match imports).
        let src = "const unused = 1;\ntype X = number;";
        let s = split_imports(src, ScriptLang::Ts, false);
        assert!(
            !s.hoisted.contains("declare const unused"),
            "no stub should be emitted for unreferenced body names:\n{}",
            s.hoisted
        );
    }

    #[test]
    fn declare_stub_skipped_for_import_names() {
        // Imported names are already at module scope; we must NOT emit
        // a `declare const foo: any;` for them (would collide with the
        // import).
        let src = "import { foo } from 'mod';\ntype X = typeof foo;";
        let s = split_imports(src, ScriptLang::Ts, false);
        assert!(
            !s.hoisted.contains("declare const foo"),
            "no stub for imported name:\n{}",
            s.hoisted
        );
        assert!(s.hoisted.contains("import { foo }"));
        assert!(s.hoisted.contains("type X"));
    }

    #[test]
    fn declare_stub_not_emitted_from_value_hoist_scanning() {
        // An import specifier containing a name that matches a body-level
        // const must NOT trigger a stub. The scan only runs on type-alias
        // / interface spans (no value shape).
        let src = "import { arr } from 'mod';\nconst arr2 = 1;\nconsole.log(arr2);\n";
        let s = split_imports(src, ScriptLang::Ts, false);
        // arr2 is a body const; `import { arr }` is a hoisted value span.
        // No hoisted type references arr2 → no stub.
        assert!(!s.hoisted.contains("declare const arr2"));
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
        let s = split_imports(src, ScriptLang::Ts, false);
        assert!(s.hoisted.contains("import { writable }"));
        assert!(!s.hoisted.contains("export { count }"));
        assert!(s.body.contains("let count = writable(0);"));
    }
}
