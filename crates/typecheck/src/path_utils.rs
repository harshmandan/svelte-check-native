//! Path math: lexical normalisation, relative-path computation, and
//! the cross-workspace `../`-import rewriter that runs over each
//! overlay before it's written to the cache.
//!
//! Mirrors upstream svelte2tsx's
//! `helpers/rewriteExternalImports.ts` for the rewrite. The other
//! helpers are pure-Rust path utilities.

use std::path::{Path, PathBuf};

/// Rewrite `../`-starting module specifiers in overlay text so they
/// resolve correctly from the overlay's location (under
/// `<workspace>/node_modules/.cache/svelte-check-native/svelte/...`)
/// instead of from the source's location.
///
/// Mirrors upstream svelte2tsx's
/// `helpers/rewriteExternalImports.ts::getExternalImportRewrite`:
/// for each module specifier, if it starts with `../` AND the resolved
/// target sits OUTSIDE `workspace`, rewrite the specifier to be relative
/// to the overlay's directory.
///
/// In-workspace `../`-imports stay as-is — they pass through TS's
/// `rootDirs` virtual mapping (the overlay tsconfig lists both the
/// source and cache directories as rootDirs).
///
/// The specifiers come from a parse of the overlay: `import` /
/// `export … from` declarations, `import()` and `require()` calls,
/// `import()` types, and `import()` inside comments (JSDoc types).
/// Nothing else is touched, so a plain string that happens to contain
/// `from "../…"` keeps its value and its literal type.
pub(crate) fn rewrite_external_imports(
    overlay_text: &str,
    source_path: &Path,
    overlay_path: &Path,
    workspace: &Path,
) -> String {
    let Some(source_dir) = source_path.parent() else {
        return overlay_text.to_string();
    };
    let Some(overlay_dir) = overlay_path.parent() else {
        return overlay_text.to_string();
    };
    // Cheap pre-filter: nothing to rewrite without a parent-dir
    // specifier somewhere in the text.
    if !overlay_text.contains("../") {
        return overlay_text.to_string();
    }

    let mut out = String::with_capacity(overlay_text.len());
    let mut copy_from = 0;
    for (start, end) in specifier_spans(overlay_text) {
        // Spans cover the quoted literal; the quotes stay in place.
        let Some(specifier) = overlay_text.get(start + 1..end - 1) else {
            continue;
        };
        if let Some(rewritten) = compute_rewrite(specifier, source_dir, overlay_dir, workspace) {
            out.push_str(&overlay_text[copy_from..start + 1]);
            out.push_str(&rewritten);
            copy_from = end - 1;
        }
    }
    out.push_str(&overlay_text[copy_from..]);
    out
}

/// Where upstream splices the external-import rewrite into a SvelteKit
/// file's typed copy: `(byte offset just past the opening quote,
/// prefix)` for each `../` specifier that leaves the workspace.
///
/// Upstream (`applyExternalImportRewritesToAddedCode`) records each
/// rewrite as an insertion — the part of the new relative path in front
/// of the user's own path — so it maps back like any other added code.
/// Same specifier set and rewrite rule as [`rewrite_external_imports`].
pub(crate) fn external_import_prefixes(
    text: &str,
    source_path: &Path,
    overlay_path: &Path,
    workspace: &Path,
) -> Vec<(usize, String)> {
    let (Some(source_dir), Some(overlay_dir)) = (source_path.parent(), overlay_path.parent())
    else {
        return Vec::new();
    };
    if !text.contains("../") {
        return Vec::new();
    }
    specifier_spans(text)
        .into_iter()
        .filter_map(|(start, end)| {
            let specifier = text.get(start + 1..end - 1)?;
            let (path_part, _) = split_specifier(specifier);
            let rewritten = compute_rewrite(specifier, source_dir, overlay_dir, workspace)?;
            let (rewritten_path, _) = split_specifier(&rewritten);
            let prefix_len = rewritten_path.len().checked_sub(path_part.len())?;
            let prefix = rewritten_path.get(..prefix_len)?;
            (!prefix.is_empty()).then(|| (start + 1, prefix.to_string()))
        })
        .collect()
}

/// Byte spans (quotes included) of every module specifier literal in
/// `text`, in source order.
fn specifier_spans(text: &str) -> Vec<(usize, usize)> {
    let alloc = oxc_allocator::Allocator::default();
    let parsed = svn_parser::parse_script_body(&alloc, text, svn_parser::ScriptLang::Ts);
    let mut probe = SpecifierSpans { spans: Vec::new() };
    oxc_ast_visit::Visit::visit_program(&mut probe, &parsed.program);
    for comment in &parsed.program.comments {
        let span = comment.content_span();
        probe.collect_from_comment(text, span.start as usize, span.end as usize);
    }
    probe.spans.sort_unstable();
    probe.spans
}

/// Upstream's `splitImportSpecifier`: `(path, ?query/#hash suffix)`.
fn split_specifier(specifier: &str) -> (&str, &str) {
    match specifier.find(['?', '#']) {
        Some(i) => (&specifier[..i], &specifier[i..]),
        None => (specifier, ""),
    }
}

/// Byte spans (quotes included) of every module specifier literal in
/// the overlay.
struct SpecifierSpans {
    spans: Vec<(usize, usize)>,
}

impl SpecifierSpans {
    fn push_literal(&mut self, lit: &oxc_ast::ast::StringLiteral<'_>) {
        self.spans
            .push((lit.span.start as usize, lit.span.end as usize));
    }

    /// `import()` / `require()` take an expression; only a plain string
    /// or a substitution-free template literal is a static specifier.
    fn push_expression(&mut self, expr: &oxc_ast::ast::Expression<'_>) {
        match expr {
            oxc_ast::ast::Expression::StringLiteral(lit) => self.push_literal(lit),
            oxc_ast::ast::Expression::TemplateLiteral(tpl) if tpl.expressions.is_empty() => {
                self.spans
                    .push((tpl.span.start as usize, tpl.span.end as usize));
            }
            _ => {}
        }
    }

    /// `import('…')` inside a comment — JSDoc `@type {import('../x').T}`.
    /// Comments have no AST, so the literal is located by text.
    fn collect_from_comment(&mut self, text: &str, start: usize, end: usize) {
        let body = &text[start..end];
        let mut from = 0;
        while let Some(rel) = body[from..].find("import(") {
            let after = from + rel + "import(".len();
            let rest = &body[after..];
            let skip = rest.len() - rest.trim_start().len();
            let quote_at = after + skip;
            let Some(quote) = body.as_bytes().get(quote_at).copied() else {
                break;
            };
            if matches!(quote, b'\'' | b'"' | b'`')
                && let Some(len) = body[quote_at + 1..].find(quote as char)
            {
                let close = quote_at + 1 + len;
                self.spans.push((start + quote_at, start + close + 1));
                from = close + 1;
            } else {
                from = quote_at;
            }
        }
    }
}

impl<'a> oxc_ast_visit::Visit<'a> for SpecifierSpans {
    fn visit_import_declaration(&mut self, it: &oxc_ast::ast::ImportDeclaration<'a>) {
        self.push_literal(&it.source);
    }

    fn visit_export_from_declaration(&mut self, it: &oxc_ast::ast::ExportFromDeclaration<'a>) {
        self.push_literal(&it.source);
    }

    fn visit_export_all_declaration(&mut self, it: &oxc_ast::ast::ExportAllDeclaration<'a>) {
        self.push_literal(&it.source);
    }

    fn visit_import_expression(&mut self, it: &oxc_ast::ast::ImportExpression<'a>) {
        self.push_expression(&it.source);
        oxc_ast_visit::walk::walk_import_expression(self, it);
    }

    fn visit_call_expression(&mut self, it: &oxc_ast::ast::CallExpression<'a>) {
        if let oxc_ast::ast::Expression::Identifier(id) = &it.callee
            && id.name == "require"
            && let Some(arg) = it.arguments.first().and_then(|a| a.as_expression())
        {
            self.push_expression(arg);
        }
        oxc_ast_visit::walk::walk_call_expression(self, it);
    }

    fn visit_ts_import_type(&mut self, it: &oxc_ast::ast::TSImportType<'a>) {
        self.push_literal(&it.source);
        oxc_ast_visit::walk::walk_ts_import_type(self, it);
    }
}

/// Compute the rewritten specifier, or `None` if no rewrite is
/// needed.
fn compute_rewrite(
    specifier: &str,
    source_dir: &Path,
    overlay_dir: &Path,
    workspace: &Path,
) -> Option<String> {
    // Mirror upstream's `splitImportSpecifier`: a query (`?`) or hash
    // (`#`) suffix is not part of the path and must survive the rewrite
    // unchanged. Rewrite only the path part, then re-append the suffix.
    let (path_part, suffix) = split_specifier(specifier);
    if !path_part.starts_with("../") {
        return None;
    }
    let target = lexical_normalise(&source_dir.join(path_part));
    if is_within(&target, workspace) {
        return None;
    }
    let rewritten_path = path_relative(overlay_dir, &target);
    let rewritten = format!(
        "{}{}",
        rewritten_path.to_string_lossy().replace('\\', "/"),
        suffix
    );
    if rewritten == specifier {
        return None;
    }
    Some(rewritten)
}

/// Compute a relative path from `from_dir` to `to_path`, mirroring
/// Node's `path.relative` semantics for our two-absolute-path inputs.
pub(crate) fn path_relative(from_dir: &Path, to_path: &Path) -> PathBuf {
    let from = lexical_normalise(from_dir);
    let to = lexical_normalise(to_path);
    let from_components: Vec<_> = from.components().collect();
    let to_components: Vec<_> = to.components().collect();
    let common_len = from_components
        .iter()
        .zip(to_components.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut out = PathBuf::new();
    for _ in 0..(from_components.len() - common_len) {
        out.push("..");
    }
    for component in &to_components[common_len..] {
        out.push(component);
    }
    out
}

/// Is `target` inside `dir` lexically?
pub(crate) fn is_within(target: &Path, dir: &Path) -> bool {
    let target_n = lexical_normalise(target);
    let dir_n = lexical_normalise(dir);
    target_n.starts_with(&dir_n)
}

/// Resolve `.` and `..` components of `p` lexically — without touching
/// the filesystem. Used to normalise tsgo's relative-with-`..` paths
/// after they've been joined onto a workspace root.
///
/// `dunce::canonicalize` would also resolve symlinks, but requires the
/// file to exist. Lexical normalisation works on virtual paths (the
/// cache may be written but tsgo's `..`-formed path may not literally
/// exist as that string). Mirrors the path-clean crate's algorithm.
pub(crate) fn lexical_normalise(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    let mut has_root = false;
    for component in p.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                has_root = true;
                out.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                let last = out.components().next_back();
                match last {
                    Some(Component::Normal(_)) => {
                        out.pop();
                    }
                    Some(Component::ParentDir) | None => {
                        // Leading `..` chain on a relative path is
                        // preserved — there's nothing to pop against.
                        out.push(component.as_os_str());
                    }
                    _ if has_root => {
                        // `..` past the root collapses to the root
                        // (Unix `cd /..` stays at `/`).
                    }
                    _ => out.push(component.as_os_str()),
                }
            }
            Component::Normal(_) => out.push(component.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{external_import_prefixes, rewrite_external_imports};

    #[test]
    fn kit_external_imports_become_prefix_insertions() {
        let ws = std::path::Path::new("/repo/app");
        let text = "import { db } from '../../../shared/db?x';\nimport a from '../lib/a';\n";
        let got = external_import_prefixes(
            text,
            &ws.join("src/routes/+page.ts"),
            &ws.join("node_modules/.cache/scn/svelte/src/routes/+page.ts"),
            ws,
        );
        // Only the specifier leaving the workspace; the prefix lands
        // just inside its opening quote.
        assert_eq!(got, vec![(20, "../../../../".to_string())]);
    }
    use std::path::Path;

    fn rewrite(overlay: &str) -> String {
        rewrite_external_imports(
            overlay,
            Path::new("/ws/src/nested/Foo.svelte"),
            Path::new(
                "/ws/node_modules/.cache/svelte-check-native/svelte/src/nested/Foo.svelte.svn.ts",
            ),
            Path::new("/ws"),
        )
    }

    #[test]
    fn module_specifiers_leaving_the_workspace_are_rebased() {
        let out = rewrite(
            "import a from '../../../ext/a';\nexport { b } from \"../../../ext/b\";\nconst c = import('../../../ext/c');\nconst d = require('../../../ext/d');\ntype E = import('../../../ext/e').E;\n",
        );
        assert!(!out.contains("'../../../ext/a'"), "{out}");
        assert!(!out.contains("\"../../../ext/b\""), "{out}");
        assert!(!out.contains("import('../../../ext/c')"), "{out}");
        assert!(!out.contains("require('../../../ext/d')"), "{out}");
        assert!(!out.contains("import('../../../ext/e')"), "{out}");
        assert_eq!(out.lines().count(), 5);
    }

    #[test]
    fn plain_strings_are_not_specifiers() {
        // The text merely mentions `from "../"`; its literal type must
        // survive untouched.
        let src = "const s = 'from \"../../../ext/x\"' as const;\nconst u = `import \"../../../ext/x\"`;\n";
        assert_eq!(rewrite(src), src);
    }

    #[test]
    fn import_equals_require_is_left_alone() {
        // Upstream rewrites `require()` calls but not `import x = require()`,
        // which is a declaration rather than a call.
        let src = "import x = require('../../../ext/x');\n";
        assert_eq!(rewrite(src), src);
    }

    #[test]
    fn in_workspace_specifiers_are_untouched() {
        let src = "import a from '../lib/a';\n";
        assert_eq!(rewrite(src), src);
    }
}
