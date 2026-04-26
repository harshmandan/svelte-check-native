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

/// Locate `svelte.config.js` / `svelte.config.mjs` / `svelte.config.ts`
/// starting from `workspace` and walking up to the filesystem root.
/// Returns the first match. Skips if it's not a regular file.
pub fn find_svelte_config(workspace: &Path) -> Option<PathBuf> {
    let mut dir = workspace.to_path_buf();
    loop {
        for candidate in ["svelte.config.js", "svelte.config.mjs", "svelte.config.ts"] {
            let p = dir.join(candidate);
            if p.is_file() {
                return Some(p);
            }
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Read `svelte.config.js` and extract `kit.files` overrides into a
/// [`KitFilesSettings`]. Returns defaults when the config is absent,
/// unparseable, or doesn't mention `kit.files`. Each path field
/// (params / hooks.{server,client,universal}) is resolved
/// independently — a partial override falls back to defaults for the
/// fields it doesn't set.
///
/// Recognised shapes:
///
/// ```js
/// export default {
///     kit: {
///         files: {
///             params: 'src/myparams',
///             hooks: 'src/myhooks',                     // legacy: universal-only
///             // or
///             hooks: { server: '…', client: '…', universal: '…' },
///         },
///     },
/// };
/// ```
///
/// Anything else (computed expressions, dynamic imports, spread)
/// falls back to defaults for the fields we couldn't resolve.
pub fn parse_kit_files_settings(config_path: &Path) -> crate::kit_files::KitFilesSettings {
    let mut settings = crate::kit_files::KitFilesSettings::default();
    let Ok(source) = std::fs::read_to_string(config_path) else {
        return settings;
    };
    let source_type = SourceType::from_path(config_path).unwrap_or_default();
    let alloc = Allocator::default();
    let parser = Parser::new(&alloc, &source, source_type);
    let parsed = parser.parse();
    let Some(files_obj) = extract_kit_files_object(&parsed.program) else {
        return settings;
    };
    apply_kit_files_overrides(files_obj, &mut settings);
    settings
}

/// Walk the program AST looking for `kit.files` inside the default
/// export. Mirrors [`extract_warning_filter`]'s shape for
/// `compilerOptions.warningFilter`.
fn extract_kit_files_object<'a>(
    program: &'a oxc_ast::ast::Program<'a>,
) -> Option<&'a ObjectExpression<'a>> {
    let mut named: std::collections::HashMap<String, &Expression<'_>> =
        std::collections::HashMap::new();
    for stmt in &program.body {
        if let Statement::VariableDeclaration(vd) = stmt {
            for d in &vd.declarations {
                let oxc_ast::ast::BindingPatternKind::BindingIdentifier(id) = &d.id.kind else {
                    continue;
                };
                if let Some(init) = &d.init {
                    named.insert(id.name.to_string(), init);
                }
            }
        }
    }
    for stmt in &program.body {
        let Statement::ExportDefaultDeclaration(decl) = stmt else {
            continue;
        };
        let expr = match &decl.declaration {
            ExportDefaultDeclarationKind::Identifier(id) => named.get(id.name.as_str()).copied()?,
            ExportDefaultDeclarationKind::ObjectExpression(obj) => {
                return kit_files_from_root(obj);
            }
            other => match other.as_expression() {
                Some(e) => e,
                None => continue,
            },
        };
        if let Expression::ObjectExpression(obj) = expr {
            return kit_files_from_root(obj);
        }
    }
    None
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
fn apply_kit_files_overrides(
    files_obj: &ObjectExpression<'_>,
    settings: &mut crate::kit_files::KitFilesSettings,
) {
    if let Some(p) = lookup_string_property(files_obj, "params") {
        settings.params_path = p;
    }
    if let Some(hooks_expr) = lookup_object_property(files_obj, "hooks") {
        match hooks_expr {
            // Legacy form: `hooks: 'src/myhooks'` → universal only.
            Expression::StringLiteral(s) => {
                settings.universal_hooks_path = s.value.to_string();
            }
            // Modern form: `hooks: { server, client, universal }`.
            Expression::ObjectExpression(hobj) => {
                if let Some(p) = lookup_string_property(hobj, "server") {
                    settings.server_hooks_path = p;
                }
                if let Some(p) = lookup_string_property(hobj, "client") {
                    settings.client_hooks_path = p;
                }
                if let Some(p) = lookup_string_property(hobj, "universal") {
                    settings.universal_hooks_path = p;
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

/// Read `svelte.config.js` and build a filter plan. Any read/parse
/// failure → empty plan (equivalent to no filter).
pub fn analyse_config(config_path: &Path) -> WarningFilterPlan {
    let Ok(source) = std::fs::read_to_string(config_path) else {
        return WarningFilterPlan::default();
    };
    let source_type = SourceType::from_path(config_path).unwrap_or_default();
    let alloc = Allocator::default();
    let parser = Parser::new(&alloc, &source, source_type);
    let parsed = parser.parse();
    // Accept programs that have parse errors as long as they carry a
    // usable program tree — `svelte.config.ts` may use syntax oxc
    // rejects (decorators etc.) but most real-world configs are
    // simple JS.
    let Some(filter_expr) = extract_warning_filter(&parsed.program) else {
        return WarningFilterPlan::default();
    };
    // `filter_expr` is the arrow / function expression; analyse it.
    let param_name = filter_param_name(filter_expr).map(str::to_string);
    let Some(param) = param_name else {
        return WarningFilterPlan::partial("could not determine filter parameter name");
    };
    analyse_filter_body(filter_expr, &param, &source)
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

/// Walk top-level statements looking for the `warningFilter` entry
/// inside `compilerOptions`. Handles two common export shapes:
///   - `export default { compilerOptions: { warningFilter: ... } }`
///   - `const config = { ... }; export default config;`
fn extract_warning_filter<'a>(
    program: &'a oxc_ast::ast::Program<'a>,
) -> Option<&'a Expression<'a>> {
    // Named declarations keyed by identifier for the second pattern.
    let mut named: std::collections::HashMap<String, &Expression<'_>> =
        std::collections::HashMap::new();
    for stmt in &program.body {
        if let Statement::VariableDeclaration(vd) = stmt {
            for d in &vd.declarations {
                let oxc_ast::ast::BindingPatternKind::BindingIdentifier(id) = &d.id.kind else {
                    continue;
                };
                if let Some(init) = &d.init {
                    named.insert(id.name.to_string(), init);
                }
            }
        }
    }
    for stmt in &program.body {
        if let Statement::ExportDefaultDeclaration(edd) = stmt {
            let expr_opt: Option<&Expression<'_>> = match &edd.declaration {
                ExportDefaultDeclarationKind::ObjectExpression(obj) => {
                    return warning_filter_in_object(obj);
                }
                ExportDefaultDeclarationKind::Identifier(id) => {
                    named.get(id.name.as_str()).copied()
                }
                _ => None,
            };
            if let Some(e) = expr_opt
                && let Expression::ObjectExpression(obj) = e
                && let Some(f) = warning_filter_in_object(obj)
            {
                return Some(f);
            }
        }
    }
    None
}

/// Look up `compilerOptions.warningFilter` inside a top-level config
/// object.
fn warning_filter_in_object<'a>(obj: &'a ObjectExpression<'a>) -> Option<&'a Expression<'a>> {
    for prop in &obj.properties {
        let ObjectPropertyKind::ObjectProperty(p) = prop else {
            continue;
        };
        let PropertyKey::StaticIdentifier(id) = &p.key else {
            continue;
        };
        if id.name != "compilerOptions" {
            continue;
        }
        // Unwrap `/** @type {...} */ ({...})` style casts.
        let mut value = &p.value;
        while let Expression::ParenthesizedExpression(px) = value {
            value = &px.expression;
        }
        let Expression::ObjectExpression(inner) = value else {
            continue;
        };
        for prop in &inner.properties {
            let ObjectPropertyKind::ObjectProperty(pp) = prop else {
                continue;
            };
            let PropertyKey::StaticIdentifier(iid) = &pp.key else {
                continue;
            };
            if iid.name == "warningFilter" {
                return Some(&pp.value);
            }
        }
    }
    None
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
    match &p.pattern.kind {
        oxc_ast::ast::BindingPatternKind::BindingIdentifier(id) => Some(id.name.as_str()),
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
    for stmt in stmts {
        match stmt {
            Statement::VariableDeclaration(vd) => {
                if let Some((name, list)) = extract_string_array_const(vd) {
                    arrays.insert(name, list);
                }
            }
            Statement::IfStatement(is) => match classify_if(is, param) {
                IfOutcome::DropWhenCondTrue(mut inner) => rules.append(&mut inner),
                IfOutcome::Unhandled => {
                    partial = true;
                    if excerpt.is_none() {
                        excerpt = Some(source_slice(source, is.span.start, is.span.end));
                    }
                }
                IfOutcome::NoEffect => {}
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
                        // Only a `return true` default matters
                        // semantically (drop-rules collected above);
                        // `return false` default drops everything not
                        // already explicitly kept — we don't model
                        // keep-rules, so treat as unknown.
                        if !b {
                            out.constant = Some(false);
                        }
                        return out;
                    }
                    // `return !ignore.includes(w.code)` shape.
                    if let Some(drops) = try_blocklist_return(arg, param, &arrays) {
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
    let oxc_ast::ast::BindingPatternKind::BindingIdentifier(id) = &d.id.kind else {
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
        analyse_config(&path)
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
