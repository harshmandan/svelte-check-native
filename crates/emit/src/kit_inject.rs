//! SvelteKit Kit-file type injection.
//!
//! Mirrors a subset of upstream svelte2tsx's `upsertKitFile` behavior:
//! for a Kit file whose user source omits the handler's parameter type,
//! splice in an `: import('./$types').Xxx` annotation. The result is
//! the original source with insertions at specific byte positions —
//! positions that line up with where the user would have hand-written
//! the annotation, so diagnostic positions map back cleanly.
//!
//! v0.3 scope (MVP):
//!
//! - `+server.ts` HTTP handlers (`GET` / `POST` / `PUT` / `PATCH` /
//!   `DELETE` / `OPTIONS` / `HEAD` / `fallback`) — inject
//!   `: import('./$types').RequestEvent` on the single untyped
//!   parameter.
//!
//! Deliberately NOT handled here (yet):
//!
//! - `+page.ts` / `+page.server.ts` / `+layout.*` `load` functions —
//!   the type name depends on Server vs universal vs Layout vs Page
//!   and has multiple variants we'd need to branch on.
//! - `actions` const satisfies pattern.
//! - `prerender` / `ssr` / `csr` / `trailingSlash` variable types.
//! - `hooks.server.ts` / `hooks.client.ts` handlers.
//! - `src/params/*.ts` param matchers.
//! - `.js` route files (needs JSDoc param annotation injection, a
//!   separate code path).
//!
//! Those round out the feature set once the MVP is proven. Each
//! category needs its own branch in `inject` + a hand-written
//! fixture per architectural rule 8.

use oxc_allocator::Allocator;
use oxc_ast::ast::{Declaration, Statement};
use oxc_span::GetSpan;
use svn_parser::{ScriptLang, parse_script_body};
use std::path::Path;

/// HTTP method names that `+server.ts` may export as handler functions,
/// per the SvelteKit runtime. Order matches upstream svelte2tsx's
/// `insertApiMethod` sequence for parity.
const SERVER_HANDLER_NAMES: &[&str] = &[
    "GET", "PUT", "POST", "PATCH", "DELETE", "OPTIONS", "HEAD", "fallback",
];

/// Returns the modified source with injected type annotations, or
/// `None` if no injections were needed (no handlers matched OR all
/// handlers already carry explicit types).
///
/// The returned string preserves the original source's byte layout
/// except at the insertion points — every insertion is purely
/// additive, so diagnostic positions at lines unaffected by the
/// inject still map 1:1 to the source.
pub fn inject(path: &Path, source: &str) -> Option<String> {
    let basename = path.file_name().and_then(|s| s.to_str())?;
    // Only `+server.ts` in this MVP. Other Kit files fall back to
    // passthrough (no injection) until the other branches land.
    if basename != "+server.ts" {
        return None;
    }

    let alloc = Allocator::default();
    let parsed = parse_script_body(&alloc, source, ScriptLang::Ts);

    let mut insertions: Vec<(usize, String)> = Vec::new();
    for stmt in &parsed.program.body {
        let Statement::ExportNamedDeclaration(export) = stmt else {
            continue;
        };
        let Some(Declaration::FunctionDeclaration(func)) = &export.declaration else {
            continue;
        };
        let Some(name) = func.id.as_ref().map(|id| id.name.as_str()) else {
            continue;
        };
        if !SERVER_HANDLER_NAMES.contains(&name) {
            continue;
        }

        // Exactly one parameter, no existing type annotation. The
        // destructure case (`{ url }`) and identifier case (`event`)
        // both have `type_annotation: None` when the user leaves it
        // implicit. Rest params and multi-param signatures are
        // deliberately skipped — they don't match the RequestEvent
        // shape upstream injects.
        if func.params.items.len() != 1 {
            continue;
        }
        let param = &func.params.items[0];
        if param.pattern.type_annotation.is_some() {
            continue;
        }

        // Splice point: immediately after the parameter pattern's
        // closing byte. For `{ url }` that's after `}`; for `event`
        // that's after `t`. Result: `function GET({ url }: Import<…>.RequestEvent)`.
        let insert_at = param.pattern.span().end as usize;
        insertions.push((
            insert_at,
            ": import('./$types').RequestEvent".to_string(),
        ));
    }

    if insertions.is_empty() {
        return None;
    }

    insertions.sort_by_key(|(pos, _)| std::cmp::Reverse(*pos));
    let mut out = source.to_string();
    for (pos, text) in insertions {
        out.insert_str(pos, &text);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn server_path() -> PathBuf {
        PathBuf::from("src/routes/+server.ts")
    }

    #[test]
    fn injects_on_destructured_single_param() {
        let source = "export async function GET({ url }) {\n    return new Response(url.pathname);\n}";
        let got = inject(&server_path(), source).unwrap();
        assert!(
            got.contains("({ url }: import('./$types').RequestEvent)"),
            "expected annotation after destructure; got:\n{got}"
        );
    }

    #[test]
    fn injects_on_identifier_param() {
        let source = "export function POST(event) { return new Response(''); }";
        let got = inject(&server_path(), source).unwrap();
        assert!(
            got.contains("(event: import('./$types').RequestEvent)"),
            "expected annotation after identifier; got:\n{got}"
        );
    }

    #[test]
    fn leaves_typed_param_alone() {
        let source = "export function GET(event: Foo) { return new Response(''); }";
        assert!(inject(&server_path(), source).is_none());
    }

    #[test]
    fn handles_multiple_handlers() {
        let source = "\
export async function GET({ url }) { return new Response(url.pathname); }
export async function POST({ request }) { return new Response(''); }
";
        let got = inject(&server_path(), source).unwrap();
        assert!(got.contains("{ url }: import('./$types').RequestEvent"));
        assert!(got.contains("{ request }: import('./$types').RequestEvent"));
    }

    #[test]
    fn skips_non_handler_exports() {
        let source = "export function helper(x) { return x; }";
        assert!(inject(&server_path(), source).is_none());
    }

    #[test]
    fn skips_multi_param_handlers() {
        // Not a valid SvelteKit handler shape; don't guess.
        let source = "export function GET(a, b) { return new Response(''); }";
        assert!(inject(&server_path(), source).is_none());
    }

    #[test]
    fn non_server_file_returns_none() {
        let source = "export async function GET({ url }) { return new Response(''); }";
        let page_path = PathBuf::from("src/routes/+page.ts");
        assert!(inject(&page_path, source).is_none());
    }

    #[test]
    fn preserves_bytes_outside_insertion() {
        let prefix = "// user comment\nexport async function GET({ url }) {";
        let suffix = "\n    return new Response(url.pathname);\n}\n";
        let source = format!("{prefix}{suffix}");
        let got = inject(&server_path(), &source).unwrap();
        // Prefix + suffix bytes should appear identically; only the
        // insertion sits between `}` (destructure close) and `)`
        // (param list close).
        assert!(got.starts_with("// user comment\n"));
        assert!(got.contains("return new Response(url.pathname);"));
    }
}
