//! Static analysis of the user's `svelte.config.js` `warningFilter`
//! callback.
//!
//! Upstream ships `warningFilter` as an arbitrary JS function in the
//! compiler options:
//!
//! ```js
//! export default {
//!   compilerOptions: {
//!     warningFilter: (w) => !w.code.startsWith('a11y_') && w.code !== 'css_unused_selector'
//!   }
//! }
//! ```
//!
//! We don't invoke the JS compiler (we parse + lint natively in
//! Rust), so there's no runtime to call this callback. Instead, we
//! parse the config file with `oxc`, extract the `warningFilter`
//! arrow, and pattern-match the body for a handful of known shapes.
//! When the body matches, we translate it into a structured
//! `WarningFilterPlan` that the CLI applies alongside the user's
//! `--compiler-warnings` flag.
//!
//! Supported patterns cover ~97% of real-world usage observed on
//! GitHub (100-sample survey logged in `notes/lint-progress.md`):
//!   - `w.code === 'x'` / `w.code !== 'x'`
//!   - `w.code.startsWith('x')` (or `.includes` / `.endsWith`)
//!   - `w.filename.includes('x')` / `w.filename?.includes('x')`
//!   - `['a','b','c'].includes(w.code)`
//!   - `const ignore = [...]; return !ignore.includes(w.code);`
//!   - Negation (`!`), conjunction (`&&`), disjunction (`||`)
//!   - Block bodies with `if (COND) return false|true; return BOOL;`
//!
//! Anything we don't recognise → emit a stderr note and fall back to
//! no filter. Users always have `--compiler-warnings code:ignore` as
//! a code-based escape hatch.

use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    ArrayExpression, ArrayExpressionElement, ArrowFunctionExpression, BinaryOperator,
    CallExpression, ChainElement, ExportDefaultDeclarationKind, Expression, FormalParameter,
    FunctionBody, IfStatement, LogicalOperator, ObjectExpression, ObjectPropertyKind, PropertyKey,
    Statement, UnaryOperator, VariableDeclaration,
};
use oxc_parser::Parser;
use oxc_span::SourceType;
use svn_core::sveltekit::{KitFilesSettings, normalise_path as normalise_kit_path};

/// Recognised filter operations. Each entry is a "drop this warning
/// if" predicate; the CLI ORs them together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropRule {
    /// Drop if `warning.code == s`.
    CodeEquals(String),
    /// Drop if `warning.code.starts_with(s)`.
    CodePrefix(String),
    /// Drop if `warning.code.contains(s)` (rare).
    CodeSubstring(String),
    /// Drop if `warning.code.ends_with(s)` (rare).
    CodeSuffix(String),
    /// Drop if `warning.filename.contains(s)` (most common: "node_modules").
    FilenameContains(String),
}

/// Outcome of analysing a `svelte.config.js`.
#[derive(Debug, Default, Clone)]
pub struct WarningFilterPlan {
    /// Drop rules to apply. Empty vec = no filtering.
    pub rules: Vec<DropRule>,
    /// `true` when a `warningFilter` was found but we couldn't parse
    /// some part of it. Caller warns the user that the filter was
    /// skipped so they know they need `--compiler-warnings`.
    pub partial: bool,
    /// `true` when the filter returns a constant (e.g. `() => false`
    /// drops everything, `() => true` keeps everything). No partial
    /// fallback — the constant IS the filter.
    pub constant: Option<bool>,
    /// For user-facing messages — the subset of the callback body we
    /// couldn't translate. `None` when the whole body parsed.
    pub unrecognised_excerpt: Option<String>,
}

impl WarningFilterPlan {
    /// Decide whether a given (code, path) pair should be dropped.
    pub fn should_drop(&self, code: &str, path: Option<&Path>) -> bool {
        if let Some(constant) = self.constant {
            return !constant;
        }
        for rule in &self.rules {
            match rule {
                DropRule::CodeEquals(s) => {
                    if code == s {
                        return true;
                    }
                }
                DropRule::CodePrefix(s) => {
                    if code.starts_with(s.as_str()) {
                        return true;
                    }
                }
                DropRule::CodeSubstring(s) => {
                    if code.contains(s.as_str()) {
                        return true;
                    }
                }
                DropRule::CodeSuffix(s) => {
                    if code.ends_with(s.as_str()) {
                        return true;
                    }
                }
                DropRule::FilenameContains(s) => {
                    if let Some(p) = path
                        && let Some(str_path) = p.to_str()
                        && str_path.contains(s.as_str())
                    {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Locate the user's svelte config in the `workspace` directory only.
/// Recognises every extension upstream svelte-check accepts: `.js`,
/// `.cjs`, `.mjs`, `.ts`, `.mts`. Returns the first match in the order
/// listed.
///
/// Probes only the given directory — it does NOT ascend to parent
/// directories. For the workspace-root `kit.files` resolution that
/// mirrors upstream svelte-check, which loads the config with
/// `loadConfig(workspacePath, { traverse: false })` (incremental.ts) —
/// the `traverse: false` flag stops the loader at the workspace dir.
/// Ascending would, in a monorepo whose sub-app has no local
/// `svelte.config` but an ancestor does, apply that ancestor's
/// `kit.files` where upstream applies defaults — shifting Kit-file
/// classification off parity. Per-file `warningFilter` / `runes`
/// resolution (which upstream DOES search per document) layers on top
/// via [`ConfigResolver`], calling this per directory.
pub fn find_svelte_config(workspace: &Path) -> Option<PathBuf> {
    for candidate in [
        "svelte.config.js",
        "svelte.config.cjs",
        "svelte.config.mjs",
        "svelte.config.ts",
        "svelte.config.mts",
    ] {
        let p = workspace.join(candidate);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Probe the workspace dir for a `vite.config.*`. Same
/// workspace-dir-only discipline as [`find_svelte_config`] (no ancestor
/// traversal) — upstream loads config with `traverse: false`.
///
/// Since SvelteKit 2.62 (svelte/kit#15944) Svelte/Kit settings can be
/// passed inline to the `sveltekit()` / `svelte()` Vite plugin instead
/// of `svelte.config.js`, and upstream svelte-check reads them from
/// there via `@sveltejs/load-config` (which runs Vite's `resolveConfig`
/// and reads `plugin.api.options`). We can't run Vite, so we statically
/// read the plugin's inline object literal — see [`analyse_vite_config`].
pub fn find_vite_config(workspace: &Path) -> Option<PathBuf> {
    for candidate in [
        "vite.config.js",
        "vite.config.ts",
        "vite.config.mjs",
        "vite.config.mts",
        "vite.config.cjs",
        "vite.config.cts",
    ] {
        let p = workspace.join(candidate);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Bundle of every static-analysis result we extract from a
/// `svelte.config.{js,mjs,cjs,ts}`. Single parse → one AST → both
/// the warning-filter plan and the kit.files overrides run against
/// the same parsed program, sharing the bumpalo allocator and the
/// SourceType detection.
#[derive(Debug, Default, Clone)]
pub struct SvelteConfigSummary {
    pub warning_filter_plan: WarningFilterPlan,
    pub kit_files_settings: KitFilesSettings,
    /// `compilerOptions.namespace === 'foreign'` — preserve DOM
    /// attribute-name case in emit (upstream `preserveAttributeCase`).
    pub preserve_attribute_case: bool,
    /// `compilerOptions.runes` boolean literal. Upstream compiles every
    /// component with the config's compilerOptions, so a config-level
    /// `runes: true` / `runes: false` FORCES the mode; `None` keeps the
    /// per-file auto-detection.
    pub runes: Option<bool>,
}

/// The per-file subset of a config — the settings upstream applies PER
/// DOCUMENT (each document compiles with its own resolved config's
/// `compilerOptions`).
#[derive(Debug, Default, Clone)]
pub struct ResolvedConfig {
    pub warning_filter_plan: WarningFilterPlan,
    /// `compilerOptions.runes` — see [`SvelteConfigSummary::runes`].
    pub runes: Option<bool>,
}

/// Per-file nearest-config resolution for `warningFilter` / `runes`.
///
/// Upstream's language server resolves each document's Svelte config by
/// searching UPWARD from the file's own directory (`Document.ts` →
/// `configLoader.awaitConfig` → `searchConfigPathUpwards`): the nearest
/// config wins outright — no merging between configs. We mirror that
/// between each `.svelte` file and the workspace root; files with no
/// nearer config fall back to the workspace-root resolution the CLI
/// already performed (which covers the `--config` override and the
/// vite-plugin-options fallback). We deliberately do NOT search above
/// the workspace root.
///
/// An explicit `--config` pins that one config for every file —
/// upstream documents that nested configs below it are ignored.
///
/// NOT resolved per-file on purpose: `kit.files` (upstream svelte-check
/// loads it with `loadConfig(workspace, { traverse: false })`, so
/// root-only IS parity) and `namespace: 'foreign'` (a process-wide emit
/// flag today).
///
/// Resolution is memoized per DIRECTORY — O(dirs), not O(files ×
/// depth). [`ConfigResolver::prime`] walks each file's ancestor chain
/// once (filesystem probes + config analysis happen here, on one
/// thread); [`ConfigResolver::for_path`] is a read-only lookup that the
/// parallel lint pass can call concurrently.
pub struct ConfigResolver {
    workspace: PathBuf,
    root: std::sync::Arc<ResolvedConfig>,
    /// `--config` was given — every file resolves to `root`.
    explicit: bool,
    by_dir: std::collections::HashMap<PathBuf, std::sync::Arc<ResolvedConfig>>,
    /// Below-root configs discovered during `prime`, keyed by config
    /// path (a config shared by several directories is analysed once).
    nested: Vec<(PathBuf, std::sync::Arc<ResolvedConfig>)>,
}

impl ConfigResolver {
    pub fn new(workspace: PathBuf, root: ResolvedConfig, explicit_config: bool) -> Self {
        Self {
            workspace,
            root: std::sync::Arc::new(root),
            explicit: explicit_config,
            by_dir: std::collections::HashMap::new(),
            nested: Vec::new(),
        }
    }

    /// Resolve (and memoize) the nearest config for every file's
    /// directory chain. Call once, before any [`Self::for_path`] use.
    pub fn prime<'a>(&mut self, files: impl IntoIterator<Item = &'a PathBuf>) {
        if self.explicit {
            // Everything resolves to the pinned config — skip the walk.
            return;
        }
        for file in files {
            if let Some(dir) = file.parent() {
                self.resolve_dir(dir);
            }
        }
    }

    fn resolve_dir(&mut self, dir: &Path) -> std::sync::Arc<ResolvedConfig> {
        if let Some(hit) = self.by_dir.get(dir) {
            return hit.clone();
        }
        let resolved = if dir == self.workspace.as_path() || !dir.starts_with(&self.workspace) {
            // The workspace root's own config was already resolved by
            // the CLI (with --config / vite fallback) — reuse it rather
            // than re-probing the root dir.
            self.root.clone()
        } else if let Some((cfg_path, summary)) = probe_dir_for_config(dir) {
            if let Some((_, existing)) = self.nested.iter().find(|(p, _)| *p == cfg_path) {
                existing.clone()
            } else {
                let rc = std::sync::Arc::new(ResolvedConfig {
                    warning_filter_plan: summary.warning_filter_plan,
                    runes: summary.runes,
                });
                self.nested.push((cfg_path, rc.clone()));
                rc
            }
        } else {
            match dir.parent() {
                Some(parent) => self.resolve_dir(parent),
                None => self.root.clone(),
            }
        };
        self.by_dir.insert(dir.to_path_buf(), resolved.clone());
        resolved
    }

    /// The nearest config for `path` — memo lookup with root fallback.
    /// Read-only; safe from parallel workers after [`Self::prime`].
    pub fn for_path(&self, path: &Path) -> &ResolvedConfig {
        if self.explicit {
            return &self.root;
        }
        let mut dir = path.parent();
        while let Some(d) = dir {
            if let Some(hit) = self.by_dir.get(d) {
                return hit;
            }
            if d == self.workspace.as_path() {
                break;
            }
            dir = d.parent();
        }
        &self.root
    }

    /// Below-root configs discovered during [`Self::prime`] whose
    /// `warningFilter` only partially translated — the caller surfaces
    /// the same stderr notice the root config gets.
    pub fn nested_partial_configs(&self) -> impl Iterator<Item = (&Path, &WarningFilterPlan)> {
        self.nested
            .iter()
            .filter(|(_, c)| c.warning_filter_plan.partial)
            .map(|(p, c)| (p.as_path(), &c.warning_filter_plan))
    }
}

/// Probe ONE directory for a Svelte/Vite config, mirroring the CLI's
/// workspace-root precedence: an analysable `vite.config.*` (inline
/// plugin options) wins, else a `svelte.config.*`, else a vite config
/// with no readable options still ENDS the upward search — upstream's
/// per-directory probe stops at any config file — contributing
/// defaults.
fn probe_dir_for_config(dir: &Path) -> Option<(PathBuf, SvelteConfigSummary)> {
    let vite = find_vite_config(dir);
    if let Some(p) = &vite
        && let Some(s) = analyse_vite_config(p)
    {
        return Some((p.clone(), s));
    }
    if let Some(p) = find_svelte_config(dir) {
        let s = analyse(&p);
        return Some((p, s));
    }
    vite.map(|p| (p, SvelteConfigSummary::default()))
}

/// Read `svelte.config.js`, parse once, and return every recognised
/// extraction (warning filter + kit.files) as a single
/// [`SvelteConfigSummary`].
///
/// Returns defaults when the config is absent, unreadable, or
/// unparseable. Each extractor runs independently against the same
/// parsed program; one extractor failing doesn't affect the other.
pub fn analyse(config_path: &Path) -> SvelteConfigSummary {
    let mut summary = SvelteConfigSummary::default();
    let Ok(source) = std::fs::read_to_string(config_path) else {
        return summary;
    };
    let source_type = SourceType::from_path(config_path).unwrap_or_default();
    let alloc = Allocator::default();
    let parser = Parser::new(&alloc, &source, source_type);
    let parsed = parser.parse();

    // Warning-filter extraction.
    if let Some(filter_expr) = extract_warning_filter(&parsed.program) {
        if let Some(param) = filter_param_name(filter_expr).map(str::to_string) {
            summary.warning_filter_plan = analyse_filter_body(filter_expr, &param, &source);
        } else {
            summary.warning_filter_plan =
                WarningFilterPlan::partial("could not determine filter parameter name");
        }
    }

    // Kit-files extraction.
    if let Some(files_obj) = extract_kit_files_object(&parsed.program) {
        apply_kit_files_overrides(files_obj, &mut summary.kit_files_settings);
    }

    // namespace: 'foreign' → preserve attribute case.
    summary.preserve_attribute_case = extract_preserve_attribute_case(&parsed.program);

    // compilerOptions.runes — config-forced runes mode.
    summary.runes =
        default_export_config_object(&parsed.program).and_then(|obj| runes_in_object(obj));

    summary
}

/// Static analysis of a `vite.config.*` — extract the Svelte/Kit
/// settings passed inline to the `sveltekit()` / `svelte()` Vite plugin
/// (SvelteKit 2.62+, svelte/kit#15944).
///
/// Returns `None` when the file is absent/unreadable/unparseable, or
/// when no statically-resolvable plugin options object literal is
/// present. That lets the caller fall back to `svelte.config.js`,
/// mirroring upstream (`@sveltejs/load-config`), which uses the Vite
/// plugin's `api.options` only when the plugin exposes them and
/// otherwise loads `svelte.config.js`.
///
/// The extraction mirrors [`analyse`] but for the one structural
/// difference documented in svelte/kit#15944: in the inline plugin form
/// the `kit` fields spread to the TOP LEVEL of the options object, so
/// `kit.files` in `svelte.config.js` becomes a top-level `files` here —
/// matching upstream's `const { preprocess, compilerOptions, extensions,
/// vitePlugin, ...kit } = pluginOptions` rest-destructure.
/// `compilerOptions` keeps the same relative position, so warningFilter
/// and namespace extraction are shared with the `svelte.config.js` path.
pub fn analyse_vite_config(config_path: &Path) -> Option<SvelteConfigSummary> {
    let source = std::fs::read_to_string(config_path).ok()?;
    let source_type = SourceType::from_path(config_path).unwrap_or_default();
    let alloc = Allocator::default();
    let parser = Parser::new(&alloc, &source, source_type);
    let parsed = parser.parse();

    let (plugin_obj, is_kit) = find_vite_svelte_plugin_options(&parsed.program)?;

    let mut summary = SvelteConfigSummary::default();

    // warningFilter — `compilerOptions.warningFilter`, identical path.
    if let Some(filter_expr) = warning_filter_in_object(plugin_obj) {
        if let Some(param) = filter_param_name(filter_expr).map(str::to_string) {
            summary.warning_filter_plan = analyse_filter_body(filter_expr, &param, &source);
        } else {
            summary.warning_filter_plan =
                WarningFilterPlan::partial("could not determine filter parameter name");
        }
    }

    // kit.files — only the SvelteKit plugin carries `files`, spread at
    // the top level of the options object (not under a `kit` key).
    if is_kit
        && let Some(files) = lookup_object_property(plugin_obj, "files")
        && let Expression::ObjectExpression(files_obj) = files
    {
        apply_kit_files_overrides(files_obj, &mut summary.kit_files_settings);
    }

    // namespace: 'foreign' → preserve attribute case.
    summary.preserve_attribute_case = preserve_case_in_object(plugin_obj);

    // compilerOptions.runes — same relative position as in
    // `svelte.config.js`.
    summary.runes = runes_in_object(plugin_obj);

    Some(summary)
}

/// Locate the inline options object passed to the `sveltekit(...)` /
/// `svelte(...)` Vite plugin inside a `vite.config.*` program, and
/// whether it is the SvelteKit plugin.
///
/// Prefers `sveltekit()` over `svelte()` (upstream reads the
/// `vite-plugin-sveltekit-setup` plugin first, then falls back to the
/// bare `vite-plugin-svelte` plugin). Returns `None` unless a plugin
/// call has a statically-resolvable object-literal first argument — a
/// bare `sveltekit()` (no args), an aliased import, or a computed
/// argument yields `None`, so we never guess: the caller falls back to
/// `svelte.config.js` exactly as upstream does when the plugin exposes
/// no inline options.
fn find_vite_svelte_plugin_options<'a>(
    program: &'a oxc_ast::ast::Program<'a>,
) -> Option<(&'a ObjectExpression<'a>, bool)> {
    let root = default_export_config_object(program)?;
    let Expression::ArrayExpression(plugins) = lookup_object_property(root, "plugins")? else {
        return None;
    };
    let mut svelte_fallback: Option<&ObjectExpression<'_>> = None;
    for element in &plugins.elements {
        let Some(expr) = element.as_expression() else {
            continue;
        };
        let Expression::CallExpression(call) = expr else {
            continue;
        };
        let Expression::Identifier(callee) = &call.callee else {
            continue;
        };
        let is_kit = match callee.name.as_str() {
            "sveltekit" => true,
            "svelte" => false,
            _ => continue,
        };
        let Some(arg) = call.arguments.first().and_then(|a| a.as_expression()) else {
            continue;
        };
        let Expression::ObjectExpression(obj) = unwrap_config_wrapper(arg) else {
            continue;
        };
        if is_kit {
            return Some((obj, true));
        }
        if svelte_fallback.is_none() {
            svelte_fallback = Some(obj);
        }
    }
    svelte_fallback.map(|obj| (obj, false))
}

/// RHS of a top-level `module.exports = …;` assignment — the CommonJS
/// equivalent of `export default` for `.cjs` (and CJS-mode `.js`)
/// configs. Upstream's loadConfig executes the config module and Node's
/// `import()` maps `module.exports` to the default export, so both
/// forms must resolve to the same config object.
fn module_exports_value<'a>(stmt: &'a Statement<'a>) -> Option<&'a Expression<'a>> {
    let Statement::ExpressionStatement(es) = stmt else {
        return None;
    };
    let Expression::AssignmentExpression(assign) = &es.expression else {
        return None;
    };
    if assign.operator != oxc_ast::ast::AssignmentOperator::Assign {
        return None;
    }
    let oxc_ast::ast::AssignmentTarget::StaticMemberExpression(target) = &assign.left else {
        return None;
    };
    let Expression::Identifier(obj) = &target.object else {
        return None;
    };
    if obj.name != "module" || target.property.name != "exports" {
        return None;
    }
    Some(&assign.right)
}

/// Find the top-level config `ObjectExpression` the file exports,
/// handling `export default {…}`, the CommonJS `module.exports = {…}`,
/// `const c = {…}; export default c` (or `module.exports = c`), and
/// `defineConfig({…})` / `satisfies` wrappers. Shared by every
/// extractor (warningFilter, kit.files, namespace, vite plugin options)
/// so all of them accept the same export shapes.
fn default_export_config_object<'a>(
    program: &'a oxc_ast::ast::Program<'a>,
) -> Option<&'a ObjectExpression<'a>> {
    let mut named: std::collections::HashMap<String, &Expression<'_>> =
        std::collections::HashMap::new();
    for stmt in &program.body {
        if let Statement::VariableDeclaration(vd) = stmt {
            for d in &vd.declarations {
                if let oxc_ast::ast::BindingPattern::BindingIdentifier(id) = &d.id
                    && let Some(init) = &d.init
                {
                    named.insert(id.name.to_string(), init);
                }
            }
        }
    }
    for stmt in &program.body {
        let expr = match stmt {
            Statement::ExportDefaultDeclaration(decl) => match &decl.declaration {
                ExportDefaultDeclarationKind::Identifier(id) => {
                    match named.get(id.name.as_str()).copied() {
                        Some(e) => e,
                        None => continue,
                    }
                }
                ExportDefaultDeclarationKind::ObjectExpression(obj) => return Some(obj),
                other => match other.as_expression() {
                    Some(e) => e,
                    None => continue,
                },
            },
            other => match module_exports_value(other) {
                Some(e) => e,
                None => continue,
            },
        };
        let unwrapped = unwrap_config_wrapper(expr);
        if let Expression::ObjectExpression(obj) = unwrapped {
            return Some(obj);
        }
        if let Expression::Identifier(id) = unwrapped
            && let Some(target) = named.get(id.name.as_str()).copied()
            && let Expression::ObjectExpression(obj) = unwrap_config_wrapper(target)
        {
            return Some(obj);
        }
    }
    None
}

/// `compilerOptions.namespace === 'foreign'` in the default-export config.
fn extract_preserve_attribute_case(program: &oxc_ast::ast::Program<'_>) -> bool {
    default_export_config_object(program)
        .map(preserve_case_in_object)
        .unwrap_or(false)
}

/// The `compilerOptions` object inside a config-root object, unwrapping
/// `/** @type {...} */ ({...})` style parenthesised casts. The
/// `compilerOptions` key sits at the same relative position in a
/// `svelte.config.js` default export and in a `sveltekit(...)` /
/// `svelte(...)` Vite-plugin options object, so every compiler-option
/// extractor shares this helper.
fn compiler_options_in_object<'a>(
    obj: &'a ObjectExpression<'a>,
) -> Option<&'a ObjectExpression<'a>> {
    let mut value = lookup_object_property(obj, "compilerOptions")?;
    while let Expression::ParenthesizedExpression(px) = value {
        value = &px.expression;
    }
    match value {
        Expression::ObjectExpression(inner) => Some(inner),
        _ => None,
    }
}

/// `compilerOptions.namespace === 'foreign'` inside a config-root object.
fn preserve_case_in_object(obj: &ObjectExpression<'_>) -> bool {
    compiler_options_in_object(obj)
        .and_then(|co| lookup_string_property(co, "namespace"))
        .as_deref()
        == Some("foreign")
}

/// `compilerOptions.runes` boolean literal inside a config-root object.
/// Non-literal values (env-dependent expressions, identifiers) yield
/// `None` — auto-detection stays in charge rather than guessing.
fn runes_in_object(obj: &ObjectExpression<'_>) -> Option<bool> {
    match lookup_object_property(compiler_options_in_object(obj)?, "runes")? {
        Expression::BooleanLiteral(b) => Some(b.value),
        _ => None,
    }
}

/// `kit.files` inside the exported config object. Export-shape
/// traversal (default export, `module.exports`, named-const
/// indirection, `defineConfig` / `satisfies` wrappers) is shared via
/// [`default_export_config_object`].
fn extract_kit_files_object<'a>(
    program: &'a oxc_ast::ast::Program<'a>,
) -> Option<&'a ObjectExpression<'a>> {
    kit_files_from_root(default_export_config_object(program)?)
}

/// Strip common one-level wrappers that don't change the underlying
/// config object: `defineConfig(X)` → `X`; `X satisfies T` → `X`;
/// `(X as T)` / `<T>X` → `X`.
fn unwrap_config_wrapper<'a>(expr: &'a Expression<'a>) -> &'a Expression<'a> {
    match expr {
        Expression::CallExpression(call) => {
            // Only unwrap recognised helpers — a generic call we
            // can't statically resolve stays opaque.
            let is_define_config = match &call.callee {
                Expression::Identifier(id) => id.name.as_str() == "defineConfig",
                _ => false,
            };
            if is_define_config && call.arguments.len() == 1 {
                if let Some(arg) = call.arguments[0].as_expression() {
                    return unwrap_config_wrapper(arg);
                }
            }
            expr
        }
        Expression::TSSatisfiesExpression(s) => unwrap_config_wrapper(&s.expression),
        Expression::TSAsExpression(a) => unwrap_config_wrapper(&a.expression),
        Expression::TSTypeAssertion(t) => unwrap_config_wrapper(&t.expression),
        Expression::ParenthesizedExpression(p) => unwrap_config_wrapper(&p.expression),
        _ => expr,
    }
}

/// Given the root config object, return its `kit.files` if present.
fn kit_files_from_root<'a>(root: &'a ObjectExpression<'a>) -> Option<&'a ObjectExpression<'a>> {
    let kit = lookup_object_property(root, "kit")?;
    let Expression::ObjectExpression(kit_obj) = kit else {
        return None;
    };
    let files = lookup_object_property(kit_obj, "files")?;
    if let Expression::ObjectExpression(files_obj) = files {
        Some(files_obj)
    } else {
        None
    }
}

/// Apply each recognised key in `files: { … }` onto `settings`.
/// String values get normalised — leading `./` and trailing `/` are
/// stripped so the suffix-match in `classify` lines up regardless of
/// how the user spelled the path.
fn apply_kit_files_overrides(files_obj: &ObjectExpression<'_>, settings: &mut KitFilesSettings) {
    if let Some(p) = lookup_string_property(files_obj, "params") {
        settings.params_path = normalise_kit_path(&p);
    }
    if let Some(hooks_expr) = lookup_object_property(files_obj, "hooks") {
        match hooks_expr {
            // Legacy form: `hooks: 'src/myhooks'` → universal only.
            Expression::StringLiteral(s) => {
                settings.universal_hooks_path = normalise_kit_path(s.value.as_str());
            }
            // Modern form: `hooks: { server, client, universal }`.
            Expression::ObjectExpression(hobj) => {
                if let Some(p) = lookup_string_property(hobj, "server") {
                    settings.server_hooks_path = normalise_kit_path(&p);
                }
                if let Some(p) = lookup_string_property(hobj, "client") {
                    settings.client_hooks_path = normalise_kit_path(&p);
                }
                if let Some(p) = lookup_string_property(hobj, "universal") {
                    settings.universal_hooks_path = normalise_kit_path(&p);
                }
            }
            _ => {}
        }
    }
}

/// Look up a string-keyed property on an ObjectExpression and return
/// the value expression. Skips computed keys, methods, getters,
/// setters, and shorthand-without-init.
fn lookup_object_property<'a>(
    obj: &'a ObjectExpression<'a>,
    key: &str,
) -> Option<&'a Expression<'a>> {
    for prop in &obj.properties {
        let ObjectPropertyKind::ObjectProperty(p) = prop else {
            continue;
        };
        let prop_key = match &p.key {
            PropertyKey::StaticIdentifier(id) => id.name.as_str(),
            PropertyKey::StringLiteral(s) => s.value.as_str(),
            _ => continue,
        };
        if prop_key == key {
            return Some(&p.value);
        }
    }
    None
}

/// Convenience: look up `key` and return its value if it's a string
/// literal; otherwise None.
fn lookup_string_property(obj: &ObjectExpression<'_>, key: &str) -> Option<String> {
    if let Expression::StringLiteral(s) = lookup_object_property(obj, key)? {
        Some(s.value.to_string())
    } else {
        None
    }
}

impl WarningFilterPlan {
    fn partial(excerpt: &str) -> Self {
        Self {
            rules: Vec::new(),
            partial: true,
            constant: None,
            unrecognised_excerpt: Some(excerpt.to_string()),
        }
    }
}

/// `compilerOptions.warningFilter` inside the exported config object.
/// Export-shape traversal (default export, `module.exports`,
/// named-const indirection, `defineConfig` / `satisfies` wrappers) is
/// shared via [`default_export_config_object`].
fn extract_warning_filter<'a>(
    program: &'a oxc_ast::ast::Program<'a>,
) -> Option<&'a Expression<'a>> {
    warning_filter_in_object(default_export_config_object(program)?)
}

/// Look up `compilerOptions.warningFilter` inside a top-level config
/// object.
fn warning_filter_in_object<'a>(obj: &'a ObjectExpression<'a>) -> Option<&'a Expression<'a>> {
    lookup_object_property(compiler_options_in_object(obj)?, "warningFilter")
}

/// Recognise `(w) => …` / `function (w) { … }` and pull the first
/// parameter's name.
fn filter_param_name<'a>(expr: &'a Expression<'a>) -> Option<&'a str> {
    match expr {
        Expression::ArrowFunctionExpression(af) => first_param_name(&af.params.items),
        Expression::FunctionExpression(fe) => first_param_name(&fe.params.items),
        _ => None,
    }
}

fn first_param_name<'a>(params: &'a [FormalParameter<'a>]) -> Option<&'a str> {
    let p = params.first()?;
    match &p.pattern {
        oxc_ast::ast::BindingPattern::BindingIdentifier(id) => Some(id.name.as_str()),
        _ => None,
    }
}

/// Analyse the callback body. Arrow with expression body is the
/// common case; block body is supported for `if/return` shapes.
fn analyse_filter_body<'a>(
    expr: &'a Expression<'a>,
    param: &str,
    source: &str,
) -> WarningFilterPlan {
    match expr {
        Expression::ArrowFunctionExpression(af) => analyse_arrow(af, param, source),
        Expression::FunctionExpression(fe) => {
            if let Some(body) = &fe.body {
                analyse_block(body, param, source)
            } else {
                WarningFilterPlan::partial("function expression with no body")
            }
        }
        _ => WarningFilterPlan::partial("filter is not a function expression"),
    }
}

fn analyse_arrow<'a>(
    af: &'a ArrowFunctionExpression<'a>,
    param: &str,
    source: &str,
) -> WarningFilterPlan {
    // Arrow expression-body: the body is a Block whose single
    // statement is a ReturnStatement wrapping the expression.
    let body = &af.body;
    if af.expression {
        // Arrow expression-body form.
        if let Some(Statement::ExpressionStatement(es)) = body.statements.first() {
            return from_keep_expr(&es.expression, param, source);
        }
        // Some versions wrap the expression-body as a ReturnStatement.
        if let Some(Statement::ReturnStatement(ret)) = body.statements.first()
            && let Some(arg) = &ret.argument
        {
            return from_keep_expr(arg, param, source);
        }
        return WarningFilterPlan::partial("empty arrow body");
    }
    analyse_block(body, param, source)
}

fn analyse_block<'a>(block: &'a FunctionBody<'a>, param: &str, source: &str) -> WarningFilterPlan {
    analyse_block_stmt(&block.statements, param, source)
}

/// Walk a block body. Model: accumulate DropRules from each `if (X)
/// return false;` clause, and record `const ignore = [...]` arrays so
/// a trailing `return !ignore.includes(w.code)` can resolve them.
///
/// Keep-style `if (COND) return true;` statements can't be modelled
/// (we only translate drop rules), and they SHIELD everything after
/// them: at runtime the keep-if executes first, so a warning matching
/// both the keep-if and a later drop source is KEPT — translating that
/// later drop would over-suppress. Once a keep-if is seen, any
/// subsequent drop source (drop-if, blocklist return, keep-expr
/// return, or a `return false` default, which drops everything the
/// keep-ifs don't keep) degrades the plan to partial/unknown —
/// conservative keep-all with the stderr notice, never over-drop.
fn analyse_block_stmt<'a>(
    stmts: &'a [Statement<'a>],
    param: &str,
    source: &str,
) -> WarningFilterPlan {
    let mut rules: Vec<DropRule> = Vec::new();
    let mut arrays: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut partial = false;
    let mut excerpt: Option<String> = None;
    // Source text of the first keep-if seen — doubles as the "a keep-if
    // shields later drops" flag and as the partial-notice excerpt.
    let mut keep_if: Option<String> = None;
    for stmt in stmts {
        match stmt {
            Statement::VariableDeclaration(vd) => {
                if let Some((name, list)) = extract_string_array_const(vd) {
                    arrays.insert(name, list);
                }
            }
            Statement::IfStatement(is) => match classify_if(is, param) {
                IfOutcome::DropWhenCondTrue(mut inner) => {
                    if keep_if.is_some() {
                        partial = true;
                        if excerpt.is_none() {
                            excerpt = keep_if.clone();
                        }
                    } else {
                        rules.append(&mut inner);
                    }
                }
                IfOutcome::Unhandled => {
                    partial = true;
                    if excerpt.is_none() {
                        excerpt = Some(source_slice(source, is.span.start, is.span.end));
                    }
                }
                IfOutcome::NoEffect => {
                    if keep_if.is_none() {
                        keep_if = Some(source_slice(source, is.span.start, is.span.end));
                    }
                }
            },
            Statement::ReturnStatement(ret) => {
                if let Some(arg) = &ret.argument {
                    // Evaluate as constant first.
                    if let Some(b) = constant_bool(arg) {
                        let mut out = WarningFilterPlan {
                            rules,
                            partial,
                            constant: None,
                            unrecognised_excerpt: excerpt,
                        };
                        // A `return false` default drops everything not
                        // explicitly kept. With no keep-ifs there IS
                        // nothing kept — the filter is constant-false
                        // (drop all; any drop-ifs above are a subset).
                        // With keep-ifs the kept subset is unmodelled →
                        // unknown, keep-all + notice.
                        if !b {
                            if let Some(k) = keep_if {
                                out.partial = true;
                                if out.unrecognised_excerpt.is_none() {
                                    out.unrecognised_excerpt = Some(k);
                                }
                            } else {
                                out.constant = Some(false);
                            }
                        }
                        return out;
                    }
                    if keep_if.is_some() {
                        // Preceding keep-if shields this return's drops.
                        partial = true;
                        if excerpt.is_none() {
                            excerpt = keep_if.clone();
                        }
                    } else if let Some(drops) = try_blocklist_return(arg, param, &arrays) {
                        // `return !ignore.includes(w.code)` shape.
                        rules.extend(drops);
                    } else {
                        // Re-evaluate as a keep-expr.
                        let sub = from_keep_expr(arg, param, source);
                        rules.extend(sub.rules);
                        if sub.partial {
                            partial = true;
                            if excerpt.is_none() {
                                excerpt = sub.unrecognised_excerpt;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    WarningFilterPlan {
        rules,
        partial,
        constant: None,
        unrecognised_excerpt: excerpt,
    }
}

enum IfOutcome {
    /// When the `if` condition holds, drop; translate to these rules.
    DropWhenCondTrue(Vec<DropRule>),
    /// The `if` either doesn't touch drops OR handles
    /// accept-cases we don't need to model.
    NoEffect,
    /// Unrecognised shape.
    Unhandled,
}

/// Classify `if (COND) return BOOL;` statements. Recognised shapes:
///   - `if (w.code === 'x') return false;` → drop code 'x'
///   - `if (COND) return true;`             → no-effect for drops
fn classify_if<'a>(is: &'a IfStatement<'a>, param: &str) -> IfOutcome {
    let Statement::ReturnStatement(ret) = &is.consequent else {
        return IfOutcome::Unhandled;
    };
    let Some(arg) = &ret.argument else {
        return IfOutcome::Unhandled;
    };
    let Some(b) = constant_bool(arg) else {
        return IfOutcome::Unhandled;
    };
    if b {
        // `if (COND) return true;` — semantically "keep if COND" which
        // doesn't add drops. If the callback has a mix of keep/drop
        // ifs we can't cleanly translate — flag that.
        return IfOutcome::NoEffect;
    }
    // `if (COND) return false;` — drop when COND is true.
    match drop_rules_from_drop_cond(&is.test, param) {
        Some(rules) => IfOutcome::DropWhenCondTrue(rules),
        None => IfOutcome::Unhandled,
    }
}

/// Translate a `(w) => EXPR` keep-expression into drop rules. A keep
/// expression returns `true` when the warning should be kept, so its
/// negation is the drop condition. We normalise via boolean algebra:
///   `!A && !B`      → drop if A OR B
///   `!A`            → drop if A
///   `A && B`        → keep both; no drops unless the leaves decode
///                      to drops (uncommon — usually for keep-case logic)
fn from_keep_expr<'a>(expr: &'a Expression<'a>, param: &str, source: &str) -> WarningFilterPlan {
    match drop_rules_from_keep_cond(expr, param) {
        Some(rules) => WarningFilterPlan {
            rules,
            partial: false,
            constant: None,
            unrecognised_excerpt: None,
        },
        None => {
            if let Some(b) = constant_bool(expr) {
                return WarningFilterPlan {
                    rules: Vec::new(),
                    partial: false,
                    constant: Some(b),
                    unrecognised_excerpt: None,
                };
            }
            WarningFilterPlan {
                rules: Vec::new(),
                partial: true,
                constant: None,
                unrecognised_excerpt: Some(source_slice(
                    source,
                    expr.span().start,
                    expr.span().end,
                )),
            }
        }
    }
}

/// Keep-condition → drop rules (negate the keep condition).
fn drop_rules_from_keep_cond<'a>(expr: &'a Expression<'a>, param: &str) -> Option<Vec<DropRule>> {
    match expr {
        Expression::ParenthesizedExpression(p) => drop_rules_from_keep_cond(&p.expression, param),
        Expression::UnaryExpression(u) if u.operator == UnaryOperator::LogicalNot => {
            // `!X`  → keep when X is false  → drop when X is true.
            drop_rules_from_drop_cond(&u.argument, param)
        }
        Expression::LogicalExpression(le) if le.operator == LogicalOperator::And => {
            // `A && B` — both must keep. Drop if A fails OR B fails.
            let a = drop_rules_from_keep_cond(&le.left, param)?;
            let b = drop_rules_from_keep_cond(&le.right, param)?;
            Some([a, b].concat())
        }
        Expression::BinaryExpression(be)
            if matches!(
                be.operator,
                BinaryOperator::StrictInequality | BinaryOperator::Inequality
            ) =>
        {
            // `w.code !== 'x'` — keep when code != x  → drop when code == x.
            translate_equality(&be.left, &be.right, param).map(|r| vec![r])
        }
        _ => None,
    }
}

/// Direct drop-condition expression (what sits inside an `if (X)
/// return false;`).
fn drop_rules_from_drop_cond<'a>(expr: &'a Expression<'a>, param: &str) -> Option<Vec<DropRule>> {
    match expr {
        Expression::ParenthesizedExpression(p) => drop_rules_from_drop_cond(&p.expression, param),
        Expression::UnaryExpression(u) if u.operator == UnaryOperator::LogicalNot => {
            // `!X` — negated drop condition. Negating again flips us
            // back to the keep-cond path.
            drop_rules_from_keep_cond(&u.argument, param)
        }
        Expression::LogicalExpression(le) if le.operator == LogicalOperator::Or => {
            // `A || B` as drop-cond: drop if either holds.
            let a = drop_rules_from_drop_cond(&le.left, param)?;
            let b = drop_rules_from_drop_cond(&le.right, param)?;
            Some([a, b].concat())
        }
        Expression::LogicalExpression(le) if le.operator == LogicalOperator::And => {
            // `A && B` as drop-cond — would require intersection. We
            // can't collapse to a single rule reliably; give up.
            None
        }
        Expression::BinaryExpression(be)
            if matches!(
                be.operator,
                BinaryOperator::StrictEquality | BinaryOperator::Equality
            ) =>
        {
            // `w.code === 'x'` → drop when code == x.
            translate_equality(&be.left, &be.right, param).map(|r| vec![r])
        }
        Expression::CallExpression(ce) => translate_call_drop(ce, param),
        Expression::ChainExpression(ch) => match &ch.expression {
            ChainElement::CallExpression(ce) => translate_call_drop(ce, param),
            _ => None,
        },
        _ => None,
    }
}

/// Match `w.code === 'x'` or `w.code !== 'x'`.
fn translate_equality<'a>(
    left: &'a Expression<'a>,
    right: &'a Expression<'a>,
    param: &str,
) -> Option<DropRule> {
    // Handle either ordering.
    if let Some(name) = member_access(left, param)
        && let Some(s) = string_literal(right)
    {
        return match name {
            "code" => Some(DropRule::CodeEquals(s.to_string())),
            _ => None,
        };
    }
    if let Some(name) = member_access(right, param)
        && let Some(s) = string_literal(left)
    {
        return match name {
            "code" => Some(DropRule::CodeEquals(s.to_string())),
            _ => None,
        };
    }
    None
}

/// Detect `w.code.startsWith('x')` / `w.code.includes('x')` /
/// `w.filename?.includes('x')` / `['x','y'].includes(w.code)`.
/// The array-includes shape expands to multiple rules.
fn translate_call_drop<'a>(ce: &'a CallExpression<'a>, param: &str) -> Option<Vec<DropRule>> {
    let Expression::StaticMemberExpression(sme) = &ce.callee else {
        return None;
    };
    let method = sme.property.name.as_str();
    let arg = ce.arguments.first().and_then(|a| a.as_expression());
    match &sme.object {
        // `w.code.X('arg')`
        Expression::StaticMemberExpression(lhs)
            if is_warning_member(&lhs.object, param) && lhs.property.name == "code" =>
        {
            let arg = arg?;
            let s = string_literal(arg)?;
            let rule = match method {
                "startsWith" => DropRule::CodePrefix(s.to_string()),
                "includes" => DropRule::CodeSubstring(s.to_string()),
                "endsWith" => DropRule::CodeSuffix(s.to_string()),
                _ => return None,
            };
            Some(vec![rule])
        }
        // `w.filename.X('arg')`
        Expression::StaticMemberExpression(lhs)
            if is_warning_member(&lhs.object, param) && lhs.property.name == "filename" =>
        {
            if method == "includes"
                && let Some(arg) = arg
                && let Some(s) = string_literal(arg)
            {
                return Some(vec![DropRule::FilenameContains(s.to_string())]);
            }
            None
        }
        // Optional-chain: `w.filename?.X('arg')`
        Expression::ChainExpression(ch) => {
            if let ChainElement::StaticMemberExpression(lhs) = &ch.expression
                && is_warning_member(&lhs.object, param)
                && lhs.property.name == "filename"
                && method == "includes"
                && let Some(arg) = arg
                && let Some(s) = string_literal(arg)
            {
                return Some(vec![DropRule::FilenameContains(s.to_string())]);
            }
            None
        }
        // `['a','b'].includes(w.code)`
        Expression::ArrayExpression(arr) if method == "includes" => {
            let arg = arg?;
            let name = member_access(arg, param)?;
            if name != "code" {
                return None;
            }
            let list = string_list_of_array(arr)?;
            Some(list.into_iter().map(DropRule::CodeEquals).collect())
        }
        _ => None,
    }
}

/// `return !ignore.includes(w.code);` where `ignore` is a known array.
fn try_blocklist_return<'a>(
    expr: &'a Expression<'a>,
    param: &str,
    arrays: &std::collections::HashMap<String, Vec<String>>,
) -> Option<Vec<DropRule>> {
    let Expression::UnaryExpression(u) = expr else {
        return None;
    };
    if u.operator != UnaryOperator::LogicalNot {
        return None;
    }
    let Expression::CallExpression(ce) = &u.argument else {
        return None;
    };
    let Expression::StaticMemberExpression(sme) = &ce.callee else {
        return None;
    };
    if sme.property.name != "includes" {
        return None;
    }
    let Expression::Identifier(id) = &sme.object else {
        return None;
    };
    let list = arrays.get(id.name.as_str())?;
    let arg = ce.arguments.first().and_then(|a| a.as_expression())?;
    let name = member_access(arg, param)?;
    if name != "code" {
        return None;
    }
    Some(list.iter().cloned().map(DropRule::CodeEquals).collect())
}

/// Extract `const NAME = ['a','b',...];` — used for the blocklist
/// pattern where a local array is checked in the return.
fn extract_string_array_const<'a>(
    vd: &'a VariableDeclaration<'a>,
) -> Option<(String, Vec<String>)> {
    let d = vd.declarations.first()?;
    let oxc_ast::ast::BindingPattern::BindingIdentifier(id) = &d.id else {
        return None;
    };
    let init = d.init.as_ref()?;
    let arr = match init {
        Expression::ArrayExpression(a) => a,
        _ => return None,
    };
    let list = string_list_of_array(arr)?;
    Some((id.name.to_string(), list))
}

fn string_list_of_array<'a>(arr: &'a ArrayExpression<'a>) -> Option<Vec<String>> {
    let mut out = Vec::with_capacity(arr.elements.len());
    for el in &arr.elements {
        let ArrayExpressionElement::StringLiteral(s) = el else {
            return None;
        };
        out.push(s.value.to_string());
    }
    Some(out)
}

/// Is `expr` the callback parameter (bare identifier)?
fn is_warning_member<'a>(expr: &'a Expression<'a>, param: &str) -> bool {
    matches!(expr, Expression::Identifier(id) if id.name == param)
}

/// `w.code` / `w.filename` → Some("code" | "filename"). Returns None
/// for other member accesses.
fn member_access<'a>(expr: &'a Expression<'a>, param: &str) -> Option<&'a str> {
    match expr {
        Expression::StaticMemberExpression(sme) if is_warning_member(&sme.object, param) => {
            Some(sme.property.name.as_str())
        }
        _ => None,
    }
}

fn string_literal<'a>(expr: &'a Expression<'a>) -> Option<&'a str> {
    match expr {
        Expression::StringLiteral(s) => Some(s.value.as_str()),
        Expression::TemplateLiteral(tl) if tl.expressions.is_empty() && tl.quasis.len() == 1 => {
            Some(
                tl.quasis[0]
                    .value
                    .cooked
                    .as_ref()
                    .map(|c| c.as_str())
                    .unwrap_or(""),
            )
        }
        _ => None,
    }
}

fn constant_bool<'a>(expr: &'a Expression<'a>) -> Option<bool> {
    match expr {
        Expression::BooleanLiteral(b) => Some(b.value),
        _ => None,
    }
}

fn source_slice(source: &str, start: u32, end: u32) -> String {
    let s = start as usize;
    let e = (end as usize).min(source.len());
    if s >= e {
        return String::new();
    }
    source[s..e].chars().take(120).collect()
}

use oxc_span::GetSpan;

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(src: &str) -> WarningFilterPlan {
        // Unique path per call so parallel tests don't race.
        let name = format!("svelte.config.{}.mjs", std::process::id());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, src).unwrap();
        analyse(&path).warning_filter_plan
    }

    fn preserve_case(src: &str) -> bool {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("svelte.config.mjs");
        std::fs::write(&path, src).unwrap();
        analyse(&path).preserve_attribute_case
    }

    fn vite_summary(src: &str) -> Option<SvelteConfigSummary> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vite.config.ts");
        std::fs::write(&path, src).unwrap();
        analyse_vite_config(&path)
    }

    #[test]
    fn vite_sveltekit_inline_warning_filter() {
        let s = vite_summary(
            r#"
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';
export default defineConfig({
  plugins: [sveltekit({
    compilerOptions: { warningFilter: (w) => w.code !== 'state_referenced_locally' }
  })]
});
"#,
        )
        .expect("inline sveltekit options must be analysed");
        assert!(!s.warning_filter_plan.partial);
        assert_eq!(
            s.warning_filter_plan.rules,
            vec![DropRule::CodeEquals("state_referenced_locally".into())]
        );
    }

    #[test]
    fn vite_sveltekit_inline_kit_files_and_namespace() {
        // In the inline plugin form the `kit` fields spread to the top
        // level, so `files` sits beside `compilerOptions` (not under a
        // `kit` key).
        let s = vite_summary(
            r#"
import { sveltekit } from '@sveltejs/kit/vite';
export default {
  plugins: [sveltekit({
    compilerOptions: { namespace: 'foreign' },
    files: { params: 'src/lib/params' }
  })]
};
"#,
        )
        .expect("inline sveltekit options must be analysed");
        assert!(s.preserve_attribute_case);
        assert_eq!(s.kit_files_settings.params_path, "src/lib/params");
    }

    #[test]
    fn vite_bare_sveltekit_no_options_falls_back() {
        // `sveltekit()` with no inline options → the plugin would load
        // svelte.config.js at runtime; statically we can't, so we return
        // None and the caller falls back to svelte.config.js.
        assert!(
            vite_summary(
                "import { sveltekit } from '@sveltejs/kit/vite';\nexport default { plugins: [sveltekit()] };"
            )
            .is_none()
        );
    }

    #[test]
    fn vite_plain_svelte_plugin_extracts_compiler_options_only() {
        // Non-Kit project: the bare `svelte()` plugin carries
        // compilerOptions but no `files`.
        let s = vite_summary(
            r#"
import { svelte } from '@sveltejs/vite-plugin-svelte';
export default {
  plugins: [svelte({
    compilerOptions: { namespace: 'foreign' },
    files: { params: 'ignored' }
  })]
};
"#,
        )
        .expect("inline svelte plugin options must be analysed");
        assert!(s.preserve_attribute_case);
        // `files` is a Kit-only concept — the bare svelte plugin's
        // `files` (if any) is not read as kit.files.
        assert_eq!(
            s.kit_files_settings,
            crate::svelte_config::KitFilesSettings::default()
        );
    }

    #[test]
    fn vite_no_svelte_plugin_returns_none() {
        assert!(
            vite_summary(
                "import react from '@vitejs/plugin-react';\nexport default { plugins: [react()] };"
            )
            .is_none()
        );
        // No plugins array at all.
        assert!(vite_summary("export default { server: { port: 3000 } };").is_none());
    }

    #[test]
    fn find_vite_config_probes_workspace() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(find_vite_config(dir.path()), None);
        let p = dir.path().join("vite.config.ts");
        std::fs::write(&p, "export default {};").unwrap();
        assert_eq!(find_vite_config(dir.path()), Some(p));
    }

    #[test]
    fn find_svelte_config_probes_workspace_only_not_ancestors() {
        // Mirror upstream's `traverse: false`: a config in an ANCESTOR
        // of the workspace must NOT be picked up. Layout:
        //   root/svelte.config.js   (ancestor — must be ignored)
        //   root/app/               (workspace — no config)
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("svelte.config.js"), "export default {};").unwrap();
        let app = root.path().join("app");
        std::fs::create_dir(&app).unwrap();

        // Config-less workspace → None, even though an ancestor has one.
        assert_eq!(find_svelte_config(&app), None);

        // A config IN the workspace dir is found.
        let local = app.join("svelte.config.js");
        std::fs::write(&local, "export default {};").unwrap();
        assert_eq!(find_svelte_config(&app), Some(local));
    }

    #[test]
    fn config_resolver_nearest_config_wins_root_is_fallback() {
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path().to_path_buf();
        let app = root.join("packages/app");
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::write(
            app.join("svelte.config.js"),
            "export default { compilerOptions: { runes: true, warningFilter: (w) => !w.code.startsWith('a11y_') } };",
        )
        .unwrap();

        let mut resolver = ConfigResolver::new(root.clone(), ResolvedConfig::default(), false);
        let nested_file = app.join("src/App.svelte");
        let root_file = root.join("lib/Root.svelte");
        resolver.prime([&nested_file, &root_file]);

        // Nearest config (packages/app) wins for files under it…
        let nested = resolver.for_path(&nested_file);
        assert_eq!(nested.runes, Some(true));
        assert!(
            nested
                .warning_filter_plan
                .should_drop("a11y_missing_attribute", None)
        );
        // …and it REPLACES the root config outright (no merging).
        let at_root = resolver.for_path(&root_file);
        assert_eq!(at_root.runes, None);
        assert!(
            !at_root
                .warning_filter_plan
                .should_drop("a11y_missing_attribute", None)
        );
        // Unprimed paths fall back to the root config.
        let stray = root.join("never/primed/X.svelte");
        assert_eq!(resolver.for_path(&stray).runes, None);
    }

    #[test]
    fn config_resolver_walks_up_to_nearest_ancestor_below_root() {
        // Config sits at packages/app; the file is two levels deeper.
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path().to_path_buf();
        let deep = root.join("packages/app/src/lib/deep");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(
            root.join("packages/app/svelte.config.js"),
            "export default { compilerOptions: { runes: true } };",
        )
        .unwrap();

        let mut resolver = ConfigResolver::new(root, ResolvedConfig::default(), false);
        let file = deep.join("Deep.svelte");
        resolver.prime([&file]);
        assert_eq!(resolver.for_path(&file).runes, Some(true));
    }

    #[test]
    fn config_resolver_explicit_config_ignores_nested() {
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path().to_path_buf();
        let app = root.join("packages/app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(
            app.join("svelte.config.js"),
            "export default { compilerOptions: { runes: true } };",
        )
        .unwrap();

        // --config pins the root resolution for every file.
        let mut resolver = ConfigResolver::new(
            root,
            ResolvedConfig {
                warning_filter_plan: WarningFilterPlan::default(),
                runes: Some(false),
            },
            true,
        );
        let file = app.join("App.svelte");
        resolver.prime([&file]);
        assert_eq!(resolver.for_path(&file).runes, Some(false));
    }

    fn summary_of(src: &str) -> SvelteConfigSummary {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("svelte.config.mjs");
        std::fs::write(&path, src).unwrap();
        analyse(&path)
    }

    #[test]
    fn compiler_options_runes_boolean_extracted() {
        assert_eq!(
            summary_of("export default { compilerOptions: { runes: true } };").runes,
            Some(true)
        );
        assert_eq!(
            summary_of("export default { compilerOptions: { runes: false } };").runes,
            Some(false)
        );
        // Absent → auto-detect.
        assert_eq!(
            summary_of("export default { compilerOptions: {} };").runes,
            None
        );
        assert_eq!(summary_of("export default {};").runes, None);
        // Non-literal (can't be evaluated statically) → auto-detect.
        assert_eq!(
            summary_of("const flag = true;\nexport default { compilerOptions: { runes: flag } };")
                .runes,
            None
        );
    }

    #[test]
    fn vite_inline_plugin_runes_extracted() {
        let s = vite_summary(
            r#"
import { sveltekit } from '@sveltejs/kit/vite';
export default { plugins: [sveltekit({ compilerOptions: { runes: true } })] };
"#,
        )
        .expect("inline sveltekit options must be analysed");
        assert_eq!(s.runes, Some(true));
    }

    fn analyse_cjs(src: &str) -> SvelteConfigSummary {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("svelte.config.cjs");
        std::fs::write(&path, src).unwrap();
        analyse(&path)
    }

    #[test]
    fn cjs_module_exports_object_literal_is_analysed() {
        // CommonJS configs assign the config to `module.exports` instead
        // of `export default` — upstream's loadConfig executes the module
        // and Node maps module.exports to the imported default export.
        let s = analyse_cjs(
            r#"
module.exports = {
  compilerOptions: {
    namespace: 'foreign',
    warningFilter: (w) => !w.code.startsWith('a11y_')
  },
  kit: { files: { params: 'src/matchers' } }
};
"#,
        );
        assert_eq!(
            s.warning_filter_plan.rules,
            vec![DropRule::CodePrefix("a11y_".into())]
        );
        assert!(!s.warning_filter_plan.partial);
        assert_eq!(s.kit_files_settings.params_path, "src/matchers");
        assert!(s.preserve_attribute_case);
    }

    #[test]
    fn cjs_module_exports_named_const_is_analysed() {
        let s = analyse_cjs(
            r#"
const config = {
  compilerOptions: { warningFilter: (w) => w.code !== 'css_unused_selector' }
};
module.exports = config;
"#,
        );
        assert_eq!(
            s.warning_filter_plan.rules,
            vec![DropRule::CodeEquals("css_unused_selector".into())]
        );
    }

    #[test]
    fn cjs_unrelated_member_assignment_is_ignored() {
        // Only `module.exports = …` counts; other member assignments
        // must not be mistaken for the config object.
        let s = analyse_cjs(
            r#"
const cache = {};
cache.exports = { compilerOptions: { namespace: 'foreign' } };
module.other = { compilerOptions: { namespace: 'foreign' } };
"#,
        );
        assert!(!s.preserve_attribute_case);
        assert!(s.warning_filter_plan.rules.is_empty());
    }

    #[test]
    fn namespace_foreign_sets_preserve_case() {
        assert!(preserve_case(
            "export default { compilerOptions: { namespace: 'foreign' } };"
        ));
        // `const config = {...}; export default config;` shape.
        assert!(preserve_case(
            "const config = { compilerOptions: { namespace: 'foreign' } };\nexport default config;"
        ));
    }

    #[test]
    fn namespace_non_foreign_or_absent_does_not_preserve() {
        assert!(!preserve_case("export default {};"));
        assert!(!preserve_case(
            "export default { compilerOptions: { namespace: 'html' } };"
        ));
        assert!(!preserve_case("export default { compilerOptions: {} };"));
    }

    #[test]
    fn equals_code_drops_single() {
        let p = plan(
            r#"
export default {
  compilerOptions: {
    warningFilter: (w) => w.code !== 'state_referenced_locally'
  }
};
"#,
        );
        assert!(!p.partial);
        assert_eq!(
            p.rules,
            vec![DropRule::CodeEquals("state_referenced_locally".into())]
        );
    }

    #[test]
    fn starts_with_a11y() {
        let p = plan(
            r#"
export default {
  compilerOptions: {
    warningFilter: (w) => !w.code.startsWith('a11y_')
  }
};
"#,
        );
        assert_eq!(p.rules, vec![DropRule::CodePrefix("a11y_".into())]);
    }

    #[test]
    fn node_modules_filename_drop() {
        let p = plan(
            r#"
export default {
  compilerOptions: {
    warningFilter: (w) => !w.filename?.includes('node_modules') && !w.code.startsWith('a11y')
  }
};
"#,
        );
        assert!(
            p.rules
                .contains(&DropRule::FilenameContains("node_modules".into()))
        );
        assert!(p.rules.contains(&DropRule::CodePrefix("a11y".into())));
    }

    #[test]
    fn block_body_if_return_false() {
        let p = plan(
            r#"
export default {
  compilerOptions: {
    warningFilter: (w) => {
      if (w.code === 'state_referenced_locally') return false;
      return true;
    }
  }
};
"#,
        );
        assert_eq!(
            p.rules,
            vec![DropRule::CodeEquals("state_referenced_locally".into())]
        );
    }

    #[test]
    fn named_const_export() {
        let p = plan(
            r#"
const config = {
  compilerOptions: {
    warningFilter: (w) => w.code !== 'x'
  }
};
export default config;
"#,
        );
        assert_eq!(p.rules, vec![DropRule::CodeEquals("x".into())]);
    }

    #[test]
    fn constant_filter_false_drops_all() {
        let p = plan(
            r#"
export default {
  compilerOptions: {
    warningFilter: (w) => false
  }
};
"#,
        );
        assert_eq!(p.constant, Some(false));
        assert!(p.should_drop("anything", None));
    }

    #[test]
    fn allowlist_keep_if_then_return_false_is_unknown_not_constant() {
        // `if (COND) return true; return false;` keeps COND-matching
        // warnings at runtime. We don't model keep-rules, so the plan
        // must fall back to unknown (partial, keep everything) — NOT
        // constant-false, which would drop the warnings the user's
        // filter explicitly keeps.
        let p = plan(
            r#"
export default {
  compilerOptions: {
    warningFilter: (w) => {
      if (w.code.startsWith('a11y_')) return true;
      return false;
    }
  }
};
"#,
        );
        assert_eq!(p.constant, None);
        assert!(p.partial);
        assert!(p.rules.is_empty());
        assert!(!p.should_drop("a11y_click_events_have_key_events", None));
        // Conservative: unknown filters keep everything (stderr notice
        // + --compiler-warnings escape hatch), never over-drop.
        assert!(!p.should_drop("css_unused_selector", None));
    }

    #[test]
    fn drop_if_then_return_false_stays_constant_false() {
        // With only drop-ifs before it, a trailing `return false`
        // genuinely drops every warning — the collected rules are a
        // subset of "drop all", so constant-false is exact.
        let p = plan(
            r#"
export default {
  compilerOptions: {
    warningFilter: (w) => {
      if (w.code === 'x') return false;
      return false;
    }
  }
};
"#,
        );
        assert_eq!(p.constant, Some(false));
        assert!(p.should_drop("anything", None));
    }

    #[test]
    fn drop_if_after_keep_if_is_shielded_and_partial() {
        // `if (A) return true; if (B) return false; return true;` — a
        // warning matching BOTH A and B is kept at runtime (A executes
        // first), so translating B into a drop rule would over-drop.
        let p = plan(
            r#"
export default {
  compilerOptions: {
    warningFilter: (w) => {
      if (w.code.startsWith('a11y_')) return true;
      if (w.code === 'css_unused_selector') return false;
      return true;
    }
  }
};
"#,
        );
        assert!(p.partial);
        assert!(p.rules.is_empty());
        assert!(!p.should_drop("css_unused_selector", None));
    }

    #[test]
    fn drop_if_before_keep_if_keeps_the_drop_rule() {
        // Drops that precede the first keep-if execute before it at
        // runtime, so they translate exactly.
        let p = plan(
            r#"
export default {
  compilerOptions: {
    warningFilter: (w) => {
      if (w.code === 'css_unused_selector') return false;
      if (w.code.startsWith('a11y_')) return true;
      return true;
    }
  }
};
"#,
        );
        assert!(!p.partial);
        assert_eq!(
            p.rules,
            vec![DropRule::CodeEquals("css_unused_selector".into())]
        );
    }

    #[test]
    fn blocklist_return_after_keep_if_is_shielded_and_partial() {
        // `if (A) return true; return !ignore.includes(w.code);` — the
        // keep-if shields ignore-list members matching A.
        let p = plan(
            r#"
export default {
  compilerOptions: {
    warningFilter: (w) => {
      if (w.code.startsWith('a11y_')) return true;
      const ignore = ['a11y_no_static_element_interactions', 'css_unused_selector'];
      return !ignore.includes(w.code);
    }
  }
};
"#,
        );
        assert!(p.partial);
        assert!(p.rules.is_empty());
        assert!(!p.should_drop("a11y_no_static_element_interactions", None));
    }

    #[test]
    fn unrecognised_callback_partial() {
        let p = plan(
            r#"
export default {
  compilerOptions: {
    warningFilter: (w) => someFn(w)
  }
};
"#,
        );
        assert!(p.partial);
        assert!(p.rules.is_empty());
        // No filter should apply when partial with no rules.
        assert!(!p.should_drop("any_code", None));
    }

    #[test]
    fn no_filter_empty_plan() {
        let p = plan(
            r#"
export default { compilerOptions: {} };
"#,
        );
        assert!(p.rules.is_empty());
        assert!(!p.partial);
    }

    #[test]
    fn should_drop_matches() {
        let p = WarningFilterPlan {
            rules: vec![
                DropRule::CodePrefix("a11y_".into()),
                DropRule::CodeEquals("css_unused_selector".into()),
                DropRule::FilenameContains("node_modules".into()),
            ],
            ..Default::default()
        };
        assert!(p.should_drop("a11y_anything", None));
        assert!(p.should_drop("css_unused_selector", None));
        assert!(!p.should_drop("state_referenced_locally", None));
        assert!(p.should_drop(
            "state_referenced_locally",
            Some(Path::new("/app/node_modules/pkg/Foo.svelte"))
        ));
    }
}
