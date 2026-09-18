//! Small, dependency-free helpers used across the emit crate:
//! line/byte position math, render-function naming, and generic-args
//! extraction. Pulled out of `lib.rs` so the main file isn't forced to
//! carry a tail of unrelated utilities.

use std::path::Path;

use smol_str::SmolStr;
use svn_parser::Document;

/// 1-based current line of a buffer that has already been written.
pub(crate) fn current_line(s: &str) -> u32 {
    1 + s.bytes().filter(|&b| b == b'\n').count() as u32
}

/// Byte offsets of the start of each line in `s`. Index 0 is the start
/// of line 1; index N is the start of line N+1 (i.e. the position just
/// past the Nth `\n`). A final sentinel equal to `s.len()` is appended
/// so `starts[line_count]` is always valid and equals the end of the
/// last line's content — lets the consumer clamp a past-EOF (line,
/// col) to the end of the buffer without bounds-checking.
pub fn compute_line_starts(s: &str) -> Vec<u32> {
    let mut starts: Vec<u32> = Vec::with_capacity(s.bytes().filter(|&b| b == b'\n').count() + 2);
    starts.push(0);
    for (idx, b) in s.bytes().enumerate() {
        if b == b'\n' {
            starts.push((idx + 1) as u32);
        }
    }
    starts.push(s.len() as u32);
    starts
}

/// 1-based line number at the given byte offset in `source`.
#[inline]
pub(crate) fn source_line_at(source: &str, offset: u32) -> u32 {
    1 + source[..offset as usize]
        .bytes()
        .filter(|&b| b == b'\n')
        .count() as u32
}

/// Count the number of complete lines in `text` (the count of `\n` plus
/// 1 if the text doesn't end with a newline and is non-empty). Used to
/// derive an end-line for line-map entries.
#[inline]
pub(crate) fn count_lines(text: &str) -> u32 {
    let nl = text.bytes().filter(|&b| b == b'\n').count() as u32;
    if text.is_empty() {
        0
    } else if text.ends_with('\n') {
        nl
    } else {
        nl + 1
    }
}

/// Workspace root that [`render_function_name`] relativizes against
/// before hashing. Set once by the CLI before any (parallel) emit.
static RENDER_HASH_ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Set the workspace root used to key `$$render_<hash>` names. Call once
/// before emit (from the CLI, alongside `set_preserve_attribute_case`).
/// Later calls are ignored — one root per process run.
pub fn set_render_hash_root(root: &Path) {
    let _ = RENDER_HASH_ROOT.set(root.to_path_buf());
}

/// Major version of the project's installed `svelte`, when known.
static SVELTE_MAJOR: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();

/// Record the installed `svelte` major version (`None` when there is no
/// install). Call once before emit; later calls are ignored.
pub fn set_svelte_major(major: Option<u32>) {
    let _ = SVELTE_MAJOR.set(major);
}

/// svelte2tsx's `svelte5Plus`, which gates the conversions only a
/// Svelte 5 install gets. Without a known install we assume Svelte 5.
pub(crate) fn svelte5_plus() -> bool {
    SVELTE_MAJOR
        .get()
        .copied()
        .flatten()
        .is_none_or(|major| major >= 5)
}

/// Whether an attribute value expression is a top-level comma sequence
/// (`{a, b}`). svelte2tsx copies attribute values into the attrs or
/// props object without parentheses, so a sequence there is read as
/// extra object members — usually a syntax error — rather than one
/// value; the emit sites leave such a value unwrapped to match.
pub(crate) fn is_sequence_expression(expr: &str) -> bool {
    let wrapped = format!("(\n{expr}\n);");
    let alloc = oxc_allocator::Allocator::default();
    let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
    if !parsed.errors.is_empty() {
        return false;
    }
    matches!(
        parsed.program.body.as_slice(),
        [oxc_ast::ast::Statement::ExpressionStatement(stmt)]
            if matches!(
                &stmt.expression,
                oxc_ast::ast::Expression::ParenthesizedExpression(p)
                    if matches!(p.expression, oxc_ast::ast::Expression::SequenceExpression(_))
            )
    )
}

/// Derive a per-file render function name. Hash of the source path
/// prevents collisions when multiple components in the same overlay
/// project would otherwise both produce `function $$render()`
/// (TS2393 "Duplicate function implementation").
///
/// The hashed key is the path RELATIVE to the workspace root (with
/// `/` separators), so the name depends only on the file's place in
/// the project — not on where the checkout lives or the host OS. That
/// keeps committed emit snapshots byte-identical across machines while
/// preserving per-file uniqueness (two files under one root can't share
/// a relative path). Files outside the root (or when no root was set,
/// e.g. unit tests) fall back to the path as given.
///
/// The hash is the first 8 hex chars of blake3 — collision-free for any
/// realistic project size.
pub(crate) fn render_function_name(source_path: &Path) -> SmolStr {
    let rel = RENDER_HASH_ROOT
        .get()
        .and_then(|root| source_path.strip_prefix(root).ok())
        .unwrap_or(source_path);
    let bytes = rel.to_string_lossy().replace('\\', "/");
    let hash = blake3::hash(bytes.as_bytes());
    let hex = hash.to_hex();
    let short = &hex.as_str()[..8];
    SmolStr::from(format!("$$render_{short}"))
}

/// Companion-class name for [`render_function_name`]. Used by the
/// class-wrapper emit path (Phase 2 / R1 of `notes/PLAN.md`) to
/// extract body-scoped Props types at module scope via
/// `ReturnType<__svn_Render_<hash><T>['props']>`.
///
/// Matches upstream svelte2tsx's `class __sveltets_Render<T>` shape
/// but with our per-file hash prefix — same reason as `$$render_<hash>`:
/// two components in the same overlay project would collide on a bare
/// `class __svn_Render { … }`.
pub(crate) fn render_class_name(render_fn_name: &str) -> SmolStr {
    let short = render_fn_name
        .strip_prefix("$$render_")
        .unwrap_or(render_fn_name);
    SmolStr::from(format!("__svn_Render_{short}"))
}

/// Extract just the type-parameter NAMES from a Svelte-5 generics
/// attribute value.
///
/// Input is what the user wrote in `<script lang="ts" generics="…">`.
/// We splice that string verbatim into the declaration site of
/// `async function $$render_<hash><…>()` and any class-wrapper
/// declaration — both expect the full parameter syntax (with `extends`
/// constraints and `= default` defaults).
///
/// At instantiation sites we need just the names: `typeof foo<T, U>`
/// not `typeof foo<T extends X, U = Y>`. The list is read back from a
/// parse of `function f<…>() {}`, the way upstream reads
/// `param.name.getText()`, so a `>` inside `=>` or a comma inside a
/// string constraint never splits a parameter. Declaration-only
/// modifiers (`const T`, `in T`, `out U`) are dropped with the rest.
///
/// A list oxc can't read at all comes back unchanged: the declaration
/// site reports the real problem in the user's own text.
pub(crate) fn generic_arg_names(generics: &str) -> String {
    let wrapped = format!("function __svn_generics<{generics}>() {{}}");
    let alloc = oxc_allocator::Allocator::default();
    let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
    let names: Vec<&str> = parsed
        .program
        .body
        .first()
        .and_then(|stmt| match stmt {
            oxc_ast::ast::Statement::FunctionDeclaration(f) => f.type_parameters.as_deref(),
            _ => None,
        })
        .map(|tps| tps.params.iter().map(|p| p.name.name.as_str()).collect())
        .unwrap_or_default();
    if names.is_empty() {
        return generics.to_string();
    }
    names.join(", ")
}

/// Extract the `generics=` attribute value from the instance `<script>`
/// if present.
///
/// Svelte 5's syntax for declaring generic type params on a component:
///
/// ```svelte
/// <script lang="ts" generics="T extends Item, K extends keyof T">
/// ```
///
/// The value is spliced verbatim into our wrapping function as
/// `function $$render<T extends Item, K extends keyof T>() { ... }` so
/// any references to `T` / `K` inside the script body resolve correctly.
/// A top-level `type NAME = $$Generic[<constraint>];` declaration.
struct DollarGenericDecl {
    name: SmolStr,
    /// Source text of the single type argument, if any.
    constraint: Option<String>,
    /// Byte span of the whole declaration in the script.
    span: std::ops::Range<usize>,
}

/// Parse the script and list its `type NAME = $$Generic[<…>];`
/// declarations in source order.
///
/// Only a parsed type alias counts, so the same text inside a comment
/// or string is not a declaration. A `$$Generic<X, Y>` with two type
/// arguments has no single-constraint meaning (upstream rejects it
/// with a transform error); the whole list is dropped so no malformed
/// generic parameter is produced.
fn dollar_generic_decls(script: &str) -> Option<Vec<DollarGenericDecl>> {
    use oxc_ast::ast::{Declaration, Statement, TSType, TSTypeName};

    // Cheap pre-filter: the parse is only worth it when the marker
    // appears somewhere in the text.
    if !script.contains("$$Generic") {
        return Some(Vec::new());
    }
    let alloc = oxc_allocator::Allocator::default();
    let parsed = svn_parser::parse_script_body(&alloc, script, svn_parser::ScriptLang::Ts);
    if parsed.panicked {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for stmt in &parsed.program.body {
        let (alias, span) = match stmt {
            Statement::TSTypeAliasDeclaration(a) => (a, a.span),
            Statement::ExportDeclaration(e) => match &e.declaration {
                Declaration::TSTypeAliasDeclaration(a) => (a, e.span),
                _ => continue,
            },
            _ => continue,
        };
        let TSType::TSTypeReference(reference) = &alias.type_annotation else {
            continue;
        };
        let TSTypeName::IdentifierReference(id) = &reference.type_name else {
            continue;
        };
        if id.name != "$$Generic" {
            continue;
        }
        let constraint = match reference.type_arguments.as_deref() {
            None => None,
            Some(args) if args.params.len() == 1 => {
                let arg = oxc_span::GetSpan::span(&args.params[0]);
                script
                    .get(arg.start as usize..arg.end as usize)
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
            }
            Some(_) => return None,
        };
        out.push(DollarGenericDecl {
            name: SmolStr::from(alias.id.name.as_str()),
            constraint,
            span: span.start as usize..span.end as usize,
        });
    }
    Some(out)
}

/// The render function's type-parameter names, and the constraint text
/// of each `type NAME = $$Generic<Constraint>` declaration (an attribute
/// list has none) — what the type-hoisting decision needs to know about
/// generics.
pub(crate) fn generic_hoist_inputs(
    generics: Option<&(SmolStr, GenericsOrigin)>,
    script: &str,
) -> (Vec<SmolStr>, Vec<String>) {
    match generics {
        None => (Vec::new(), Vec::new()),
        Some((list, GenericsOrigin::Attribute)) => (
            generic_arg_names(list)
                .split(',')
                .map(|n| SmolStr::from(n.trim()))
                .filter(|n| !n.is_empty())
                .collect(),
            Vec::new(),
        ),
        Some((_, GenericsOrigin::DollarGeneric)) => {
            let decls = dollar_generic_decls(script).unwrap_or_default();
            let names = decls.iter().map(|d| d.name.clone()).collect();
            let constraints = decls.into_iter().filter_map(|d| d.constraint).collect();
            (names, constraints)
        }
    }
}

/// Blank out `type NAME = $$Generic[<args>];` declarations from a
/// script body, replacing each declaration with whitespace of equal
/// length so subsequent line/column source maps stay aligned.
///
/// Used in the rewrite chain when `synthesise_generics_from_dollar_generic`
/// has lifted these declarations into the render-fn's generic-param
/// list — the body must NOT also re-declare them (TS2300 duplicate
/// identifier in the function scope, on top of the local declaration
/// shadowing the generic parameter and degrading binding precision).
pub(crate) fn blank_dollar_generic_decls(script: &str) -> String {
    let mut out = script.to_string();
    for decl in dollar_generic_decls(script).unwrap_or_default() {
        let replacement: String = script[decl.span.clone()]
            .chars()
            .map(|c| if c == '\n' || c == '\r' { c } else { ' ' })
            .collect();
        out.replace_range(decl.span, &replacement);
    }
    out
}

/// Where a render function's generic-parameter list came from.
///
/// The distinction decides whether the emitted `<...>` is treated as
/// user-written text or as scaffolding: an attribute's parameters are
/// something the user typed and can be diagnosed about, whereas a list
/// synthesised from `$$Generic` declarations exists nowhere in the
/// source, so diagnostics about it (e.g. "declared but never used")
/// would point at a construct the user cannot see or fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GenericsOrigin {
    /// Copied from the `generics="..."` attribute on `<script>`.
    Attribute,
    /// Synthesised from `type NAME = $$Generic[<args>];` declarations.
    DollarGeneric,
}

pub(crate) fn extract_generics_attr(doc: &Document<'_>) -> Option<(SmolStr, GenericsOrigin)> {
    let script = doc.instance_script.as_ref()?;
    if let Some(g) = script.generics.as_deref() {
        return Some((SmolStr::from(g), GenericsOrigin::Attribute));
    }
    // SVELTE-4-COMPAT: when no `<script generics="...">` attribute is
    // present, fall back to scanning for `type NAME = $$Generic[<args>];`
    // declarations and synthesise a generic-parameter list. Mirrors
    // upstream svelte2tsx's `Generics.ts` which threads `$$Generic`
    // type names through to the render fn's `<...>` so consumer-side
    // `<Comp prop={value}>` calls bind the generic from the prop's
    // type. Without this, `type A = $$Generic` resolves at module
    // scope as `$$Generic<any> = any`, defeating per-call binding.
    synthesise_generics_from_dollar_generic(script.content)
        .map(|g| (g, GenericsOrigin::DollarGeneric))
}

/// Turn the script's `type NAME = $$Generic[<args>];` declarations into
/// a generic-parameter list. Each declaration becomes one parameter:
///   `type A = $$Generic;`            → `A`
///   `type B = $$Generic<keyof A>;`   → `B extends keyof A`
///   `type C = $$Generic<boolean>;`   → `C extends boolean`
///
/// Walk order is source order, so parameters reference each other as
/// in the user's source (`B extends keyof A` requires A first).
/// Returns `None` when no `$$Generic` declarations exist (caller's
/// non-`<script generics>` path keeps its existing behaviour).
fn synthesise_generics_from_dollar_generic(script: &str) -> Option<SmolStr> {
    let params: Vec<String> = dollar_generic_decls(script)?
        .into_iter()
        .map(|d| match d.constraint {
            Some(c) => format!("{} extends {c}", d.name),
            None => d.name.to_string(),
        })
        .collect();
    if params.is_empty() {
        None
    } else {
        Some(SmolStr::from(params.join(", ")))
    }
}

/// True for CSS-custom-property attribute names (`--foo`, `--some-var`).
/// Svelte 5 treats `<Comp --css-var={...}>` as a CSS variable on the
/// component's wrapper element, not as a typed prop — so the emit
/// routes these through `__svn_css_prop` which returns `{}` and
/// doesn't contribute to the Props object type.
pub(crate) fn is_css_custom_prop_name(name: &str) -> bool {
    name.starts_with("--")
}

/// True for ASCII identifiers `[A-Za-z_$][A-Za-z0-9_$]*`. We don't try
/// to enumerate JS reserved words — modern JS (ES5+) allows reserved
/// words as bare property names anyway.
pub(crate) fn is_simple_js_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

#[cfg(test)]
mod generic_arg_names_tests {
    use super::generic_arg_names;

    #[test]
    fn strips_constraints_and_defaults() {
        assert_eq!(generic_arg_names("T extends string, U = number"), "T, U");
    }

    #[test]
    fn strips_declaration_modifiers() {
        // `const` / `in` / `out` are declaration-only; instantiation
        // sites take the bare name (upstream `param.name.getText()`).
        assert_eq!(generic_arg_names("const T extends readonly string[]"), "T");
        assert_eq!(generic_arg_names("in T, out U"), "T, U");
        assert_eq!(generic_arg_names("const T"), "T");
    }

    #[test]
    fn plain_names_unchanged() {
        assert_eq!(generic_arg_names("T, U, V"), "T, U, V");
    }

    #[test]
    fn preserves_non_ascii_name_bytes() {
        assert_eq!(generic_arg_names("Ψ extends number"), "Ψ");
    }
}
