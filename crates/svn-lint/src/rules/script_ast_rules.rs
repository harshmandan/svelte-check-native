//! JS/TS AST rules that fire inside `<script>` blocks.
//!
//! Script bodies are parsed once via `svn-parser::parse_script_body`
//! (oxc). We walk the resulting AST with a minimal visitor that
//! tracks `function_depth` (so we can tell "class declaration
//! outside the instance scope" from "class declaration in a helper
//! function"), plus the distinction between module-script and
//! instance-script.
//!
//! Upstream equivalents:
//! - `perf_avoid_inline_class` → `visitors/NewExpression.js:11`
//! - `perf_avoid_nested_class` → `visitors/ClassDeclaration.js:21`
//! - `reactive_declaration_invalid_placement` → `visitors/LabeledStatement.js:90`

use oxc_allocator::Allocator;
use oxc_ast::ast::ClassBody;
use oxc_ast::ast::{Expression, LabeledStatement, NewExpression, Statement};
use smol_str::SmolStr;
use svn_core::Range;
use svn_parser::document::{Document, ScriptSection};
use svn_parser::parse_script_body;

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;

/// Entry point: parse both script sections and run JS-AST-dependent
/// rules on them.
pub fn visit_document(doc: &Document<'_>, ctx: &mut LintContext<'_>) {
    if let Some(script) = &doc.instance_script {
        run_on_section(script, ScriptAstContext::Instance, ctx);
    }
    if let Some(script) = &doc.module_script {
        run_on_section(script, ScriptAstContext::Module, ctx);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScriptAstContext {
    Instance,
    Module,
}

fn run_on_section(
    script: &ScriptSection<'_>,
    script_ctx: ScriptAstContext,
    ctx: &mut LintContext<'_>,
) {
    // Parse the script body with oxc.
    let lang = script.lang;
    let alloc = Allocator::default();
    // Parser needs the content string to live at least as long as the
    // allocator; `script.content` is borrowed from the source.
    let parsed = parse_script_body(&alloc, script.content, lang);
    // Script starts at script.content_range.start in the parent
    // source — but oxc gives us offsets relative to the content
    // string. We apply a uniform bias at emit time.
    let base_offset = script.content_range.start;

    let runes = ctx.runes;
    // Upstream's function_depth convention: module-script top level
    // starts at 0; instance-script top level starts at 1 (the
    // implicit component function). Match that so our T3/perf
    // thresholds line up byte-for-byte with upstream.
    let starting_depth = match script_ctx {
        ScriptAstContext::Module => 0,
        ScriptAstContext::Instance => 1,
    };

    // Pre-scan leading `// svelte-ignore …` comments in the script
    // body so visit_labeled can match on a statement's leading
    // comment. Oxc exposes comments on the program.
    let ignore_comments: Vec<IgnoreComment> = parsed
        .program
        .comments
        .iter()
        .filter_map(|c| {
            let text = &script.content[c.span.start as usize..c.span.end as usize];
            // Oxc's comment span INCLUDES the `//` / `/* */`
            // delimiters — strip them for matching.
            let body = strip_comment_delimiters(text)?;
            let trimmed = body.trim_start();
            let rest = trimmed.strip_prefix("svelte-ignore")?;
            // Require a whitespace char after the keyword.
            let rest = match rest.chars().next() {
                Some(ch) if ch.is_whitespace() => &rest[ch.len_utf8()..],
                _ => return None,
            };
            let codes = crate::ignore::parse_ignore_codes_public(rest, runes);
            Some(IgnoreComment {
                span_end: c.span.end,
                codes,
            })
        })
        .collect();

    let mut walker = ScriptWalker {
        ctx,
        script_ctx,
        base_offset,
        function_depth: starting_depth,
        runes,
        ignore_comments,
    };
    for stmt in &parsed.program.body {
        walker.visit_stmt(stmt);
    }
}

/// Matches upstream's `regex_bidirectional_control_characters`.
fn has_bidi_char(s: &str) -> bool {
    s.chars()
        .any(|c| matches!(c as u32, 0x202A..=0x202E | 0x2066..=0x2069))
}

/// Strip leading `//` or `/* */` delimiters from a raw comment source.
fn strip_comment_delimiters(text: &str) -> Option<&str> {
    if let Some(rest) = text.strip_prefix("//") {
        Some(rest)
    } else if let Some(rest) = text.strip_prefix("/*") {
        Some(rest.trim_end_matches("*/"))
    } else {
        None
    }
}

#[derive(Debug, Clone)]
struct IgnoreComment {
    /// Byte offset (within the script body, pre-bias) of the
    /// comment's END. Pairs with a statement whose span begins at
    /// or just after this offset.
    span_end: u32,
    codes: Vec<SmolStr>,
}

struct ScriptWalker<'a, 'src> {
    ctx: &'a mut LintContext<'src>,
    script_ctx: ScriptAstContext,
    base_offset: u32,
    function_depth: u32,
    runes: bool,
    /// Pre-collected `// svelte-ignore …` comments in the script,
    /// sorted by increasing `span_end`. Used to decide whether a
    /// given statement has a matching leading ignore directive.
    ignore_comments: Vec<IgnoreComment>,
}

impl<'a, 'src> ScriptWalker<'a, 'src> {
    fn visit_stmt(&mut self, stmt: &Statement<'_>) {
        match stmt {
            // Classes.
            Statement::ClassDeclaration(cls) => {
                // perf_avoid_nested_class: runes mode only. Allowed
                // at function_depth 0 in module scripts, at
                // function_depth 1 (component scope) in instance
                // scripts. Fires above that.
                //
                // Upstream logic:
                //   allowed_depth = ast_type === 'module' ? 0 : 1;
                //   if (scope.function_depth > allowed_depth) w.perf_avoid_nested_class(node);
                //
                // Our `function_depth` convention: 0 at the
                // program top level, incremented on entering a
                // function. Module scripts stay at 0 at top level;
                // instance-script top-level is ALSO 0 (the
                // component scope is a virtual implicit function in
                // upstream — we need to model that).
                if self.runes {
                    // Upstream: allowed_depth = (ast_type === 'module') ? 0 : 1.
                    // We've biased function_depth so module top is
                    // 0 and instance top is 1, matching upstream.
                    let allowed = match self.script_ctx {
                        ScriptAstContext::Module => 0,
                        ScriptAstContext::Instance => 1,
                    };
                    if self.function_depth > allowed {
                        let range = self.abs_range(cls.span.start, cls.span.end);
                        let msg = messages::perf_avoid_nested_class();
                        self.ctx.emit(Code::perf_avoid_nested_class, msg, range);
                    }
                }
                self.visit_class_body(&cls.body);
            }

            Statement::FunctionDeclaration(f) => {
                self.function_depth += 1;
                if let Some(body) = &f.body {
                    for s in &body.statements {
                        self.visit_stmt(s);
                    }
                }
                self.function_depth -= 1;
            }

            Statement::LabeledStatement(lbl) => {
                self.visit_labeled(lbl);
            }

            Statement::BlockStatement(block) => {
                for s in &block.body {
                    self.visit_stmt(s);
                }
            }

            Statement::IfStatement(i) => {
                self.visit_stmt(&i.consequent);
                if let Some(alt) = &i.alternate {
                    self.visit_stmt(alt);
                }
                self.visit_expr(&i.test);
            }
            Statement::ForStatement(f) => {
                self.visit_stmt(&f.body);
            }
            Statement::WhileStatement(w) => self.visit_stmt(&w.body),
            Statement::DoWhileStatement(d) => self.visit_stmt(&d.body),
            Statement::TryStatement(t) => {
                for s in &t.block.body {
                    self.visit_stmt(s);
                }
                if let Some(h) = &t.handler {
                    for s in &h.body.body {
                        self.visit_stmt(s);
                    }
                }
                if let Some(f) = &t.finalizer {
                    for s in &f.body {
                        self.visit_stmt(s);
                    }
                }
            }
            Statement::ExpressionStatement(es) => {
                self.check_legacy_component_creation(&es.expression);
                self.visit_expr(&es.expression);
            }
            Statement::ReturnStatement(r) => {
                if let Some(arg) = &r.argument {
                    self.visit_expr(arg);
                }
            }
            Statement::VariableDeclaration(vd) => {
                for decl in &vd.declarations {
                    if let Some(init) = &decl.init {
                        self.visit_expr(init);
                    }
                }
            }
            _ => {}
        }
    }

    fn visit_expr(&mut self, expr: &Expression<'_>) {
        match expr {
            Expression::StringLiteral(lit) => {
                if has_bidi_char(&lit.value) {
                    let range = self.abs_range(lit.span.start, lit.span.end);
                    let msg = messages::bidirectional_control_characters();
                    self.ctx
                        .emit(Code::bidirectional_control_characters, msg, range);
                }
            }
            Expression::TemplateLiteral(tl) => {
                for q in &tl.quasis {
                    if let Some(cooked) = q.value.cooked.as_deref() {
                        if has_bidi_char(cooked) {
                            let range = self.abs_range(q.span.start, q.span.end);
                            let msg = messages::bidirectional_control_characters();
                            self.ctx
                                .emit(Code::bidirectional_control_characters, msg, range);
                        }
                    }
                }
                for e in &tl.expressions {
                    self.visit_expr(e);
                }
            }
            Expression::NewExpression(ne) => self.visit_new(ne),
            Expression::ArrowFunctionExpression(arr) => {
                self.function_depth += 1;
                for s in &arr.body.statements {
                    self.visit_stmt(s);
                }
                self.function_depth -= 1;
            }
            Expression::FunctionExpression(f) => {
                self.function_depth += 1;
                if let Some(body) = &f.body {
                    for s in &body.statements {
                        self.visit_stmt(s);
                    }
                }
                self.function_depth -= 1;
            }
            Expression::ClassExpression(cls) => {
                // Nested class inside expression position. Rule
                // fires via perf_avoid_inline_class on the parent
                // NewExpression (not here); we still walk the body
                // to catch further nested rules.
                self.visit_class_body(&cls.body);
            }
            Expression::CallExpression(call) => {
                for a in &call.arguments {
                    if let Some(e) = a.as_expression() {
                        self.visit_expr(e);
                    }
                }
            }
            Expression::ParenthesizedExpression(p) => self.visit_expr(&p.expression),
            Expression::AssignmentExpression(a) => self.visit_expr(&a.right),
            Expression::BinaryExpression(b) => {
                self.visit_expr(&b.left);
                self.visit_expr(&b.right);
            }
            _ => {}
        }
    }

    fn visit_class_body(&mut self, body: &ClassBody<'_>) {
        for member in &body.body {
            use oxc_ast::ast::ClassElement;
            match member {
                ClassElement::MethodDefinition(m) => {
                    if let Some(body) = &m.value.body {
                        self.function_depth += 1;
                        for s in &body.statements {
                            self.visit_stmt(s);
                        }
                        self.function_depth -= 1;
                    }
                }
                ClassElement::PropertyDefinition(p) => {
                    if let Some(v) = &p.value {
                        self.visit_expr(v);
                    }
                }
                _ => {}
            }
        }
    }

    fn visit_new(&mut self, ne: &NewExpression<'_>) {
        // perf_avoid_inline_class: `new (class {...})` at any
        // function_depth > 0. Upstream fires only when `callee` is
        // a ClassExpression.
        if self.function_depth > 0
            && let Expression::ClassExpression(_) = &ne.callee
        {
            let range = self.abs_range(ne.span.start, ne.span.end);
            let msg = messages::perf_avoid_inline_class();
            self.ctx.emit(Code::perf_avoid_inline_class, msg, range);
        }
        // Continue walking — inner callee might be another
        // expression with nested classes.
        self.visit_expr(&ne.callee);
        for a in &ne.arguments {
            if let Some(e) = a.as_expression() {
                self.visit_expr(e);
            }
        }
    }

    /// Does any `// svelte-ignore …` comment ending before `span_start`
    /// (and separated only by whitespace) mention `code`?
    fn has_leading_ignore(&self, span_start: u32, code: &str) -> bool {
        for c in &self.ignore_comments {
            if c.span_end > span_start {
                continue;
            }
            // Check the gap between c.span_end and span_start in
            // the script source is whitespace-only.
            let content = &self.ctx.source[self.base_offset as usize..];
            let rel_start = c.span_end as usize;
            let rel_end = span_start as usize;
            if rel_end > content.len() || rel_start > rel_end {
                continue;
            }
            let gap = &content[rel_start..rel_end];
            if gap.chars().all(char::is_whitespace) && c.codes.iter().any(|k| k.as_str() == code) {
                return true;
            }
        }
        false
    }

    fn visit_labeled(&mut self, lbl: &LabeledStatement<'_>) {
        // reactive_declaration_invalid_placement: `$:` that's not
        // at the program top level of the INSTANCE script. Upstream
        // fires this outside runes mode only — but the error path
        // inside runes ignores the label. We match the warning
        // semantics: non-runes, `$:` below Program body is invalid.
        if lbl.label.name == "$" {
            // Upstream `LabeledStatement.js:90`:
            //   if (!analysis.runes) { w.reactive_declaration_invalid_placement(node); }
            // inside the "not at top level" branch.
            //
            // Our function_depth is biased so instance top is 1.
            // Fires in two cases:
            //   - Inside a function (depth > instance-top or module-top)
            //   - Inside the MODULE script at any depth (`$:` in module
            //     script is always invalid — module-script root equivalent
            //     to instance-script "not top level" in upstream's eyes).
            let instance_top = match self.script_ctx {
                ScriptAstContext::Module => 0,
                ScriptAstContext::Instance => 1,
            };
            let not_at_top = self.function_depth > instance_top;
            let in_module = self.script_ctx == ScriptAstContext::Module;
            if (!self.runes) && (not_at_top || in_module) {
                // Honour a `// svelte-ignore reactive_declaration_invalid_placement`
                // (or its legacy dashed equivalent) preceding this
                // `$:` statement inside the same script body.
                let is_ignored = self.has_leading_ignore(
                    lbl.span.start,
                    Code::reactive_declaration_invalid_placement.as_str(),
                );
                if !is_ignored {
                    let range = self.abs_range(lbl.span.start, lbl.span.end);
                    let msg = messages::reactive_declaration_invalid_placement();
                    self.ctx
                        .emit(Code::reactive_declaration_invalid_placement, msg, range);
                }
            }
        }

        // Recurse into the label's body.
        self.visit_stmt(&lbl.body);
    }

    fn abs_range(&self, start: u32, end: u32) -> Range {
        Range::new(start + self.base_offset, end + self.base_offset)
    }

    /// Upstream `visitors/ExpressionStatement.js`. Fires on
    /// `new ComponentName({ target: … })` when `ComponentName` is a
    /// default import from a `.svelte` file — the Svelte 4 class-
    /// instantiation pattern that no longer works in Svelte 5.
    fn check_legacy_component_creation(&mut self, expr: &Expression<'_>) {
        let Expression::NewExpression(ne) = expr else {
            return;
        };
        // Callee must be a bare identifier.
        let Expression::Identifier(callee) = &ne.callee else {
            return;
        };
        // Exactly one argument — an ObjectExpression with a `target`
        // property. `new Foo()` and `new Foo({})` deliberately skip.
        if ne.arguments.len() != 1 {
            return;
        }
        let Some(arg) = ne.arguments[0].as_expression() else {
            return;
        };
        let Expression::ObjectExpression(obj) = arg else {
            return;
        };
        let has_target = obj.properties.iter().any(|p| {
            matches!(
                p,
                oxc_ast::ast::ObjectPropertyKind::ObjectProperty(op)
                    if matches!(&op.key, oxc_ast::ast::PropertyKey::StaticIdentifier(k) if k.name.as_str() == "target")
            )
        });
        if !has_target {
            return;
        }
        // Callee must resolve to a default import from a `.svelte`
        // source.
        let Some(tree) = &self.ctx.scope_tree else {
            return;
        };
        let name = callee.name.as_str();
        let Some(bid) = tree.resolve_from_template(name) else {
            return;
        };
        let b = tree.binding(bid);
        let matches = matches!(
            &b.initial,
            crate::scope::InitialKind::Import { source, is_default: true }
                if source.ends_with(".svelte")
        );
        if !matches {
            return;
        }
        let range = self.abs_range(ne.span.start, ne.span.end);
        let msg = crate::messages::legacy_component_creation();
        self.ctx
            .emit(crate::codes::Code::legacy_component_creation, msg, range);
    }
}
