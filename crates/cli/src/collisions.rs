//! `.svelte` import-specifier rewriting for the `Foo.svelte` /
//! `Foo.svelte.{ts,js}` sibling-collision case.
//!
//! When a user `.ts` or `.js` file imports `./Foo.svelte` and a
//! `Foo.svelte.{ts,js}` runes module sits beside it, tsgo's
//! `rootDirs` resolver picks the user's source tree (longest
//! matching prefix), then auto-extends `.svelte` to `.svelte.{ts,js}`
//! and lands on the runes module — which has named exports but no
//! `default`, firing TS2305. Rewriting the import specifier to
//! `.svelte.svn.{js,mjs}` in an overlay sidesteps the auto-extension
//! entirely; tsgo resolves via bundler module resolution straight to
//! the cache-side overlay.
//!
//! Detection runs through oxc — the byte-scan it replaced could
//! match `from './Foo.svelte'` text inside string literals or block
//! comments and rewrite them by accident. The parser-based approach
//! only sees real import/export source positions.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast::Statement;
use svn_parser::{ScriptLang, parse_script_body};

/// Walk a user `.ts` / `.js` file's import/export source positions
/// and rewrite each `./<rel>.svelte` specifier to `./<rel>.svelte.svn.js`
/// when (and only when) a sibling runes module
/// (`./<rel>.svelte.{ts,js}`) is in `runes_modules`.
///
/// Returns `Some(rewritten)` if at least one specifier was rewritten,
/// `None` otherwise — caller skips creating an overlay in the latter
/// case (the file type-checks via the normal `include` glob).
///
/// Scope is deliberately narrow: only relative specifiers ending in
/// the literal `.svelte` extension. Non-relative imports
/// (`@foo/bar.svelte`, aliased `$lib/x.svelte`) bypass this path —
/// those either resolve via `paths` aliases in the overlay tsconfig
/// or aren't subject to the rootDirs collision. Static imports/
/// re-exports only; `import('./X.svelte')` dynamic imports are rare
/// enough in user `.ts` files to defer.
pub(crate) fn rewrite_svelte_imports_for_collisions(
    file: &Path,
    source: &str,
    runes_modules: &HashSet<PathBuf>,
) -> Option<String> {
    let file_dir = file.parent()?;
    // Parse with the script-lang the file's extension implies; both TS
    // and JS share import/export syntax for our purposes (the source-
    // string literal in an ImportDeclaration is the same node either
    // way).
    let lang = match file.extension().and_then(|s| s.to_str()) {
        Some("js" | "mjs" | "cjs") => ScriptLang::Js,
        _ => ScriptLang::Ts,
    };
    let alloc = Allocator::default();
    let parsed = parse_script_body(&alloc, source, lang);

    let mut rewrites: Vec<(usize, usize, String)> = Vec::new();
    for stmt in &parsed.program.body {
        let (literal_start, literal_end, spec_value) = match stmt {
            Statement::ImportDeclaration(decl) => (
                decl.source.span.start as usize,
                decl.source.span.end as usize,
                decl.source.value.as_str(),
            ),
            Statement::ExportNamedDeclaration(decl) => match &decl.source {
                Some(s) => (s.span.start as usize, s.span.end as usize, s.value.as_str()),
                None => continue,
            },
            Statement::ExportAllDeclaration(decl) => (
                decl.source.span.start as usize,
                decl.source.span.end as usize,
                decl.source.value.as_str(),
            ),
            _ => continue,
        };
        if !spec_value.ends_with(".svelte")
            || !(spec_value.starts_with("./") || spec_value.starts_with("../"))
        {
            continue;
        }
        // Resolve the relative target against the importing file's
        // directory; collision requires BOTH siblings — the .svelte
        // component AND a `.svelte.{ts,js}` runes module beside it.
        // A standalone runes module without a matching component
        // (the Svelte-5 pure-TS-store pattern) means the user's
        // `import './foo.svelte'` is INTENTIONALLY resolving via
        // bundler auto-extension to the runes module.
        let target = file_dir.join(spec_value);
        if !target.is_file() {
            continue;
        }
        let basename = target.file_name().and_then(|s| s.to_str())?;
        let candidate_ts = target.with_file_name(format!("{basename}.ts"));
        let candidate_js = target.with_file_name(format!("{basename}.js"));
        let canon = |p: PathBuf| dunce::canonicalize(&p).ok().unwrap_or(p);
        if !runes_modules.contains(&canon(candidate_ts))
            && !runes_modules.contains(&canon(candidate_js))
        {
            continue;
        }
        // Preserve the original quote style by replacing the WHOLE
        // string literal (quotes included) — the literal's span
        // covers exactly that range, and the source bytes at
        // span.start tell us which quote char to emit.
        let quote = source
            .as_bytes()
            .get(literal_start)
            .copied()
            .unwrap_or(b'"') as char;
        rewrites.push((
            literal_start,
            literal_end,
            format!("{quote}{spec_value}.svn.js{quote}"),
        ));
    }
    if rewrites.is_empty() {
        return None;
    }
    // Apply rewrites in reverse byte order so earlier offsets stay
    // valid while later splices happen first.
    rewrites.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    let mut out = source.to_string();
    for (start, end, replacement) in rewrites {
        out.replace_range(start..end, &replacement);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(dir: &Path, rel: &str, content: &str) -> PathBuf {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn rewrites_import_with_sibling_ts_runes_module() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "Foo.svelte", "<script>let x = 1;</script>");
        let runes = write(tmp.path(), "Foo.svelte.ts", "export const HELP = 1;");
        let importer = write(tmp.path(), "user.ts", "import Foo from './Foo.svelte';\n");
        let runes_set: HashSet<PathBuf> =
            std::iter::once(dunce::canonicalize(&runes).unwrap()).collect();
        let source = fs::read_to_string(&importer).unwrap();
        let out = rewrite_svelte_imports_for_collisions(&importer, &source, &runes_set)
            .expect("collision rewrite expected");
        assert!(out.contains("./Foo.svelte.svn.js"));
    }

    #[test]
    fn rewrites_import_with_sibling_js_runes_module() {
        // F10: .svelte.js runes modules also create the same
        // collision; the rewrite must trigger on either sibling.
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "Bar.svelte", "<script>let y = 2;</script>");
        let runes = write(tmp.path(), "Bar.svelte.js", "export const HELP = 1;");
        let importer = write(tmp.path(), "user.js", "import Bar from './Bar.svelte';\n");
        let runes_set: HashSet<PathBuf> =
            std::iter::once(dunce::canonicalize(&runes).unwrap()).collect();
        let source = fs::read_to_string(&importer).unwrap();
        let out = rewrite_svelte_imports_for_collisions(&importer, &source, &runes_set)
            .expect("collision rewrite expected for JS importer");
        assert!(out.contains("./Bar.svelte.svn.js"));
    }

    #[test]
    fn does_not_rewrite_string_literal_inside_template_literal() {
        // F11 regression: the previous byte scan matched
        // `from './Foo.svelte'` text inside string literals and
        // template-literal contents. The parser-based approach must
        // only see real import sources.
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "Foo.svelte", "<script>let x = 1;</script>");
        let runes = write(tmp.path(), "Foo.svelte.ts", "export const HELP = 1;");
        let importer = write(
            tmp.path(),
            "user.ts",
            "const docs = `example: from './Foo.svelte'`;\nexport {};\n",
        );
        let runes_set: HashSet<PathBuf> =
            std::iter::once(dunce::canonicalize(&runes).unwrap()).collect();
        let source = fs::read_to_string(&importer).unwrap();
        let out = rewrite_svelte_imports_for_collisions(&importer, &source, &runes_set);
        // No real import → no rewrite.
        assert!(out.is_none(), "must not rewrite text inside literals");
    }

    #[test]
    fn rewrites_re_export_source() {
        // `export { default } from './Foo.svelte'` re-exports go
        // through the same collision; the parser-based path picks
        // them up via ExportNamedDeclaration's source field.
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "Foo.svelte", "<script>let x = 1;</script>");
        let runes = write(tmp.path(), "Foo.svelte.ts", "export const HELP = 1;");
        let importer = write(
            tmp.path(),
            "barrel.ts",
            "export { default as Foo } from './Foo.svelte';\n",
        );
        let runes_set: HashSet<PathBuf> =
            std::iter::once(dunce::canonicalize(&runes).unwrap()).collect();
        let source = fs::read_to_string(&importer).unwrap();
        let out = rewrite_svelte_imports_for_collisions(&importer, &source, &runes_set)
            .expect("re-export rewrite expected");
        assert!(out.contains("./Foo.svelte.svn.js"));
    }
}
