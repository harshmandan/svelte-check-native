//! SlotHandler PLAN Stage 3: AST-level rewrite of `<slot {EXPR}>`
//! attribute expressions whose root identifier is bound in the
//! template scope.
//!
//! Two outputs:
//!
//! - `Some(ResolvedSlotExpr::Type(s))` — the expression was
//!   rewritten to a type-level form; emit splices it as
//!   `undefined as any as (s)`.
//! - `None` — either the expression has no shadowed root (caller
//!   splices source verbatim), or it touches a shadowed-but-
//!   unresolvable name / a shape we don't know how to rewrite at
//!   type level (bail; caller drops the attr).
//!
//! Shapes handled:
//!
//! - Bare identifier `item` → `<resolved>`.
//! - Member expression `item.foo.bar` → `<resolved>['foo']['bar']`
//!   (bracket-notation conversion is required because at TS type
//!   level `T.foo` only works for namespaces).
//! - Computed member `item['foo']` → `<resolved>['foo']` (already
//!   bracket form, just splice).
//! - Mixed chain `item.foo['bar'].baz` → `<resolved>['foo']['bar']['baz']`.
//!
//! Out of scope (returns `None`):
//!
//! - Optional chaining (`item?.foo`).
//! - Function calls, ternaries, binary operators.
//! - Object expressions, array expressions.
//! - Anything whose root isn't an Identifier (parenthesised
//!   expressions, `this`, etc.).
//!
//! Mirrors upstream `SlotHandler.resolveExpression` in
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/slot.ts:195-250`,
//! adapted to TS-type-level output instead of value-level.
//!
//! Round-7 follow-up #1: a second entry point
//! [`rewrite_slot_attr_expr_value`] walks the WHOLE expression AST
//! and rewrites every shadowed identifier in-place, leaving the
//! surrounding expression intact. Mirrors upstream's
//! `resolveExpression` byte-replace pass: each identifier in
//! non-member, non-key, non-shorthand position is replaced with its
//! resolved value (cast through `undefined as any as TYPE` for
//! type-resolved bindings); shorthand identifiers in object literals
//! get expanded to `key: replacement`. Output is a value-level
//! expression that splices verbatim into the slot literal — keeps
//! patterns like `foo(item)`, `{ item }`, `fallback ?? item`,
//! `items.map(item => item.x)` typed correctly.

use oxc_allocator::Allocator;
use oxc_ast::ast::{Expression, Statement};
use svn_parser::{ScriptLang, parse_script_body};

/// Rewrite `text` (the source slice of a slot-attr expression) into
/// a type-level form when its root identifier resolves through
/// `lookup`. See module-level docs for the exact rules.
///
/// `lookup(name)` returns:
///   - `Some(Some(resolved))` — substitute identifier with `resolved`.
///   - `Some(None)` — shadowed but not resolvable; bail.
///   - `None` — not in scope; not our identifier (let parent handle).
pub fn rewrite_slot_attr_expr(
    text: &str,
    lookup: &impl Fn(&str) -> Option<Option<String>>,
) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Wrap the expression in a let-init slot so oxc parses it as a
    // single Expression statement. The wrapper prefix `const _x = `
    // adds 12 bytes; not used for span arithmetic since we work on
    // the parsed AST directly.
    let alloc = Allocator::default();
    let wrapped = format!("const _x = ({trimmed});");
    let parsed = parse_script_body(&alloc, &wrapped, ScriptLang::Ts);
    if parsed.panicked {
        return None;
    }
    let stmt = parsed.program.body.first()?;
    let Statement::VariableDeclaration(decl) = stmt else {
        return None;
    };
    let declarator = decl.declarations.first()?;
    let init = declarator.init.as_ref()?;
    // Strip the wrapping parens we added.
    let inner = match init {
        Expression::ParenthesizedExpression(p) => &p.expression,
        other => other,
    };
    rewrite_expression(inner, lookup)
}

fn rewrite_expression(
    expr: &Expression<'_>,
    lookup: &impl Fn(&str) -> Option<Option<String>>,
) -> Option<String> {
    match expr {
        Expression::Identifier(id) => {
            let name = id.name.as_str();
            match lookup(name) {
                Some(Some(resolved)) => Some(format!("({resolved})")),
                // Shadowed but unresolvable — bail.
                Some(None) => None,
                // Module-scope identifier — caller handles via the
                // verbatim-source path. Returning None here also
                // makes the caller skip; that's intentional for
                // Stage 3, since module identifiers shouldn't
                // appear inside slot attrs whose root is shadowed.
                None => None,
            }
        }
        Expression::StaticMemberExpression(me) => {
            let base = rewrite_member_base(&me.object, lookup)?;
            Some(format!("{base}[{:?}]", me.property.name.as_str()))
        }
        Expression::ComputedMemberExpression(me) => {
            // `item['foo']` — bracket already; the property
            // expression must itself be a literal string for the
            // type-level rewrite to be valid (TS `T[K]` where `K`
            // is a string-literal type).
            let base = rewrite_member_base(&me.object, lookup)?;
            let Expression::StringLiteral(prop) = &me.expression else {
                // Computed access with a non-literal key — type-level
                // would need `T[typeof key]` projection; bail.
                return None;
            };
            Some(format!("{base}[{:?}]", prop.value.as_str()))
        }
        // Optional chains, calls, binary ops, etc. are all
        // type-level invalid for our slot-def shape — bail.
        _ => None,
    }
}

/// Walk a member-expression's `object` recursively until we hit an
/// Identifier (the chain root). If the root resolves, build the
/// bracket-notation chain on the way back up. Otherwise bail.
fn rewrite_member_base(
    object: &Expression<'_>,
    lookup: &impl Fn(&str) -> Option<Option<String>>,
) -> Option<String> {
    match object {
        Expression::Identifier(id) => {
            let name = id.name.as_str();
            match lookup(name) {
                Some(Some(resolved)) => Some(format!("({resolved})")),
                _ => None,
            }
        }
        Expression::StaticMemberExpression(me) => {
            let base = rewrite_member_base(&me.object, lookup)?;
            Some(format!("{base}[{:?}]", me.property.name.as_str()))
        }
        Expression::ComputedMemberExpression(me) => {
            let base = rewrite_member_base(&me.object, lookup)?;
            let Expression::StringLiteral(prop) = &me.expression else {
                return None;
            };
            Some(format!("{base}[{:?}]", prop.value.as_str()))
        }
        _ => None,
    }
}

/// Three-state result for the value-level walker.
#[derive(Debug, PartialEq, Eq)]
pub enum ValueRewrite {
    /// At least one shadowed identifier was rewritten. The string is
    /// the rewritten expression source.
    Rewritten(String),
    /// No shadowed identifiers were found anywhere in the expression
    /// — the caller should splice the original source verbatim.
    NoEdits,
    /// A shadowed-but-unresolvable identifier was found (or the AST
    /// was malformed) — the caller should drop the slot attr.
    Bailed,
}

/// Round-7 follow-up #1: walk the WHOLE expression AST and rewrite
/// each shadowed identifier in place. Returns a value-level
/// expression suitable for splicing into the slot literal.
///
/// `lookup(name)` returns the same shape as
/// [`rewrite_slot_attr_expr`]:
/// - `Some(Some(resolved))` — replace identifier with this expression
///   (we wrap as `(undefined as any as (resolved))` for type-resolved
///   bindings — `lookup` callers always pass type-level resolutions
///   here, so the cast is safe).
/// - `Some(None)` — shadowed but unresolvable. Walker bails.
/// - `None` — module-scope identifier. Leave unchanged.
///
/// Skips identifier sites that aren't real value references:
/// - Member-expression property positions (`x.foo` — `foo` is a name,
///   not an identifier reference).
/// - Object-key positions (`{ foo: x }` — `foo` is a key).
///
/// Object-shorthand positions (`{ item }`) get expanded to
/// `{ item: <replacement> }` — the identifier sits in BOTH key and
/// value positions in the source, but only the value reference is
/// replaced.
///
/// On any unrecognised AST node containing identifiers (e.g. JSX),
/// returns whatever was found in walked nodes (no panics, no partial
/// rewrites). The walk is conservative: missing a rewrite always
/// degrades to module-scope semantics, never to a wrong type.
pub fn rewrite_slot_attr_expr_value(
    text: &str,
    lookup: &impl Fn(&str) -> Option<Option<String>>,
) -> ValueRewrite {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return ValueRewrite::Bailed;
    }
    let alloc = Allocator::default();
    let wrapper = format!("const _x = ({trimmed});");
    let parsed = parse_script_body(&alloc, &wrapper, ScriptLang::Ts);
    if parsed.panicked {
        return ValueRewrite::Bailed;
    }
    let Some(stmt) = parsed.program.body.first() else {
        return ValueRewrite::Bailed;
    };
    let Statement::VariableDeclaration(decl) = stmt else {
        return ValueRewrite::Bailed;
    };
    let Some(declarator) = decl.declarations.first() else {
        return ValueRewrite::Bailed;
    };
    let Some(init) = declarator.init.as_ref() else {
        return ValueRewrite::Bailed;
    };
    let inner = match init {
        Expression::ParenthesizedExpression(p) => &p.expression,
        other => other,
    };
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut bail = false;
    let mut shadowed: Vec<String> = Vec::new();
    walk_value_expr(inner, lookup, &mut shadowed, &mut edits, &mut bail, false);
    if bail {
        return ValueRewrite::Bailed;
    }
    if edits.is_empty() {
        return ValueRewrite::NoEdits;
    }
    let prefix_len = "const _x = (".len();
    let mut out = trimmed.to_string();
    edits.sort_by_key(|(start, _, _)| *start);
    for w in edits.windows(2) {
        if w[0].1 > w[1].0 {
            return ValueRewrite::Bailed;
        }
    }
    for (wrap_start, wrap_end, replacement) in edits.into_iter().rev() {
        let Some(start) = wrap_start.checked_sub(prefix_len) else {
            return ValueRewrite::Bailed;
        };
        let Some(end) = wrap_end.checked_sub(prefix_len) else {
            return ValueRewrite::Bailed;
        };
        if end > out.len() || start > end {
            return ValueRewrite::Bailed;
        }
        out.replace_range(start..end, &replacement);
    }
    ValueRewrite::Rewritten(out)
}

fn walk_value_expr(
    expr: &Expression<'_>,
    lookup: &impl Fn(&str) -> Option<Option<String>>,
    // Round-12 follow-up #3: stack of identifier names introduced by
    // enclosing arrow/function expressions' params. When walking
    // their bodies, identifiers whose name is in this stack are NOT
    // rewritten — the param shadows the outer template binding.
    shadowed: &mut Vec<String>,
    edits: &mut Vec<(usize, usize, String)>,
    bail: &mut bool,
    inside_skip: bool,
) {
    if *bail {
        return;
    }
    match expr {
        Expression::Identifier(id) => {
            if inside_skip {
                return;
            }
            let name = id.name.as_str();
            // Round-12 #3: skip identifiers shadowed by an
            // enclosing arrow/function param.
            if shadowed.iter().any(|s| s == name) {
                return;
            }
            match lookup(name) {
                Some(Some(resolved)) => {
                    let start = id.span.start as usize;
                    let end = id.span.end as usize;
                    edits.push((start, end, format!("(undefined as any as ({resolved}))")));
                }
                Some(None) => {
                    *bail = true;
                }
                None => {}
            }
        }
        Expression::ParenthesizedExpression(p) => {
            walk_value_expr(&p.expression, lookup, shadowed, edits, bail, inside_skip);
        }
        Expression::StaticMemberExpression(me) => {
            walk_value_expr(&me.object, lookup, shadowed, edits, bail, false);
            // me.property is the static name — never resolved against shadow.
        }
        Expression::ComputedMemberExpression(me) => {
            walk_value_expr(&me.object, lookup, shadowed, edits, bail, false);
            walk_value_expr(&me.expression, lookup, shadowed, edits, bail, false);
        }
        Expression::CallExpression(call) => {
            walk_value_expr(&call.callee, lookup, shadowed, edits, bail, false);
            for arg in &call.arguments {
                if let Some(arg_expr) = arg.as_expression() {
                    walk_value_expr(arg_expr, lookup, shadowed, edits, bail, false);
                }
            }
        }
        Expression::ConditionalExpression(c) => {
            walk_value_expr(&c.test, lookup, shadowed, edits, bail, false);
            walk_value_expr(&c.consequent, lookup, shadowed, edits, bail, false);
            walk_value_expr(&c.alternate, lookup, shadowed, edits, bail, false);
        }
        Expression::LogicalExpression(b) => {
            walk_value_expr(&b.left, lookup, shadowed, edits, bail, false);
            walk_value_expr(&b.right, lookup, shadowed, edits, bail, false);
        }
        Expression::BinaryExpression(b) => {
            walk_value_expr(&b.left, lookup, shadowed, edits, bail, false);
            walk_value_expr(&b.right, lookup, shadowed, edits, bail, false);
        }
        Expression::UnaryExpression(u) => {
            walk_value_expr(&u.argument, lookup, shadowed, edits, bail, false);
        }
        Expression::TemplateLiteral(tl) => {
            for e in &tl.expressions {
                walk_value_expr(e, lookup, shadowed, edits, bail, false);
            }
        }
        Expression::ObjectExpression(obj) => {
            for prop in &obj.properties {
                use oxc_ast::ast::ObjectPropertyKind;
                match prop {
                    ObjectPropertyKind::ObjectProperty(p) => {
                        // Computed keys (`{[k]: v}`) recurse through the key expression.
                        if p.computed {
                            if let Some(key_expr) = p.key.as_expression() {
                                walk_value_expr(key_expr, lookup, shadowed, edits, bail, false);
                            }
                        }
                        // Shorthand `{ item }`: the value AST node sits at the same span
                        // as the key. Expand to `key: replacement` so a single span carries
                        // both identifiers cleanly.
                        if p.shorthand {
                            if let Expression::Identifier(id) = &p.value {
                                let name = id.name.as_str();
                                if shadowed.iter().any(|s| s == name) {
                                    continue;
                                }
                                if let Some(resolved) = lookup(name) {
                                    match resolved {
                                        Some(resolved_text) => {
                                            let start = id.span.start as usize;
                                            let end = id.span.end as usize;
                                            edits.push((
                                                start,
                                                end,
                                                format!(
                                                    "{name}: (undefined as any as ({resolved_text}))"
                                                ),
                                            ));
                                        }
                                        None => {
                                            *bail = true;
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                        walk_value_expr(&p.value, lookup, shadowed, edits, bail, false);
                    }
                    ObjectPropertyKind::SpreadProperty(sp) => {
                        walk_value_expr(&sp.argument, lookup, shadowed, edits, bail, false);
                    }
                }
            }
        }
        Expression::ArrayExpression(arr) => {
            for elem in &arr.elements {
                use oxc_ast::ast::ArrayExpressionElement;
                match elem {
                    ArrayExpressionElement::SpreadElement(sp) => {
                        walk_value_expr(&sp.argument, lookup, shadowed, edits, bail, false);
                    }
                    ArrayExpressionElement::Elision(_) => {}
                    other => {
                        if let Some(e) = other.as_expression() {
                            walk_value_expr(e, lookup, shadowed, edits, bail, false);
                        }
                    }
                }
            }
        }
        Expression::ChainExpression(c) => {
            use oxc_ast::ast::ChainElement;
            match &c.expression {
                ChainElement::CallExpression(call) => {
                    walk_value_expr(&call.callee, lookup, shadowed, edits, bail, false);
                    for arg in &call.arguments {
                        if let Some(arg_expr) = arg.as_expression() {
                            walk_value_expr(arg_expr, lookup, shadowed, edits, bail, false);
                        }
                    }
                }
                ChainElement::StaticMemberExpression(me) => {
                    walk_value_expr(&me.object, lookup, shadowed, edits, bail, false);
                }
                ChainElement::ComputedMemberExpression(me) => {
                    walk_value_expr(&me.object, lookup, shadowed, edits, bail, false);
                    walk_value_expr(&me.expression, lookup, shadowed, edits, bail, false);
                }
                _ => {}
            }
        }
        // Round-12 follow-up #3: walk arrow/function bodies with the
        // function's param names pushed onto the shadow stack so
        // outer-template-locals get rewritten while same-name params
        // are left alone. ClassExpression and Generator/Async forms
        // still skip — class bodies have method scoping that's
        // harder to handle uniformly.
        Expression::ArrowFunctionExpression(arrow) => {
            let before = shadowed.len();
            for param in &arrow.params.items {
                collect_pattern_idents(&param.pattern, shadowed);
            }
            for stmt in &arrow.body.statements {
                walk_statement_for_value_rewrite(stmt, lookup, shadowed, edits, bail);
            }
            shadowed.truncate(before);
        }
        Expression::FunctionExpression(fe) => {
            let before = shadowed.len();
            for param in &fe.params.items {
                collect_pattern_idents(&param.pattern, shadowed);
            }
            if let Some(body) = &fe.body {
                for stmt in &body.statements {
                    walk_statement_for_value_rewrite(stmt, lookup, shadowed, edits, bail);
                }
            }
            shadowed.truncate(before);
        }
        Expression::ClassExpression(_) => {}
        // Type-only wrappers — recurse.
        Expression::TSAsExpression(t) => {
            walk_value_expr(&t.expression, lookup, shadowed, edits, bail, false);
        }
        Expression::TSNonNullExpression(t) => {
            walk_value_expr(&t.expression, lookup, shadowed, edits, bail, false);
        }
        Expression::TSTypeAssertion(t) => {
            walk_value_expr(&t.expression, lookup, shadowed, edits, bail, false);
        }
        // Anything else (literals, JSX, tagged templates, sequences,
        // assignment, await, yield, regex, etc.): skip without bailing.
        // Identifiers that appear inside these unhandled shapes won't
        // get rewritten — degrades to module-scope semantics, not a
        // wrong type.
        _ => {}
    }
}

/// Round-12 #3: walk a statement looking for expressions that
/// might reference shadowed template locals. Arrow body shorthand
/// (single-expression body) is handled by walking the expression
/// directly. Statement bodies (block) descend through return /
/// expression / variable-decl init / control-flow.
fn walk_statement_for_value_rewrite(
    stmt: &oxc_ast::ast::Statement<'_>,
    lookup: &impl Fn(&str) -> Option<Option<String>>,
    shadowed: &mut Vec<String>,
    edits: &mut Vec<(usize, usize, String)>,
    bail: &mut bool,
) {
    use oxc_ast::ast::Statement;
    if *bail {
        return;
    }
    match stmt {
        Statement::ExpressionStatement(es) => {
            walk_value_expr(&es.expression, lookup, shadowed, edits, bail, false);
        }
        Statement::ReturnStatement(rs) => {
            if let Some(arg) = &rs.argument {
                walk_value_expr(arg, lookup, shadowed, edits, bail, false);
            }
        }
        Statement::VariableDeclaration(decl) => {
            for d in &decl.declarations {
                let before = shadowed.len();
                collect_pattern_idents(&d.id, shadowed);
                if let Some(init) = &d.init {
                    walk_value_expr(init, lookup, shadowed, edits, bail, false);
                }
                let _ = before;
            }
        }
        Statement::BlockStatement(b) => {
            for s in &b.body {
                walk_statement_for_value_rewrite(s, lookup, shadowed, edits, bail);
            }
        }
        Statement::IfStatement(s) => {
            walk_value_expr(&s.test, lookup, shadowed, edits, bail, false);
            walk_statement_for_value_rewrite(&s.consequent, lookup, shadowed, edits, bail);
            if let Some(alt) = &s.alternate {
                walk_statement_for_value_rewrite(alt, lookup, shadowed, edits, bail);
            }
        }
        // Round-13 follow-up #5: extend coverage so closed-over
        // template locals inside loops/try/switch/function-decl get
        // rewritten too. Pre-fix native bailed silently and the
        // identifiers leaked to module scope.
        Statement::ForStatement(s) => {
            if let Some(init) = &s.init {
                use oxc_ast::ast::ForStatementInit;
                match init {
                    ForStatementInit::VariableDeclaration(decl) => {
                        for d in &decl.declarations {
                            collect_pattern_idents(&d.id, shadowed);
                            if let Some(d_init) = &d.init {
                                walk_value_expr(d_init, lookup, shadowed, edits, bail, false);
                            }
                        }
                    }
                    other => {
                        if let Some(expr) = other.as_expression() {
                            walk_value_expr(expr, lookup, shadowed, edits, bail, false);
                        }
                    }
                }
            }
            if let Some(test) = &s.test {
                walk_value_expr(test, lookup, shadowed, edits, bail, false);
            }
            if let Some(update) = &s.update {
                walk_value_expr(update, lookup, shadowed, edits, bail, false);
            }
            walk_statement_for_value_rewrite(&s.body, lookup, shadowed, edits, bail);
        }
        Statement::ForInStatement(s) => {
            walk_value_expr(&s.right, lookup, shadowed, edits, bail, false);
            walk_statement_for_value_rewrite(&s.body, lookup, shadowed, edits, bail);
        }
        Statement::ForOfStatement(s) => {
            walk_value_expr(&s.right, lookup, shadowed, edits, bail, false);
            walk_statement_for_value_rewrite(&s.body, lookup, shadowed, edits, bail);
        }
        Statement::WhileStatement(s) => {
            walk_value_expr(&s.test, lookup, shadowed, edits, bail, false);
            walk_statement_for_value_rewrite(&s.body, lookup, shadowed, edits, bail);
        }
        Statement::DoWhileStatement(s) => {
            walk_statement_for_value_rewrite(&s.body, lookup, shadowed, edits, bail);
            walk_value_expr(&s.test, lookup, shadowed, edits, bail, false);
        }
        Statement::SwitchStatement(s) => {
            walk_value_expr(&s.discriminant, lookup, shadowed, edits, bail, false);
            for case in &s.cases {
                if let Some(test) = &case.test {
                    walk_value_expr(test, lookup, shadowed, edits, bail, false);
                }
                for stmt in &case.consequent {
                    walk_statement_for_value_rewrite(stmt, lookup, shadowed, edits, bail);
                }
            }
        }
        Statement::TryStatement(s) => {
            for stmt in &s.block.body {
                walk_statement_for_value_rewrite(stmt, lookup, shadowed, edits, bail);
            }
            if let Some(handler) = &s.handler {
                let before = shadowed.len();
                if let Some(param) = &handler.param {
                    collect_pattern_idents(&param.pattern, shadowed);
                }
                for stmt in &handler.body.body {
                    walk_statement_for_value_rewrite(stmt, lookup, shadowed, edits, bail);
                }
                shadowed.truncate(before);
            }
            if let Some(finalizer) = &s.finalizer {
                for stmt in &finalizer.body {
                    walk_statement_for_value_rewrite(stmt, lookup, shadowed, edits, bail);
                }
            }
        }
        Statement::LabeledStatement(s) => {
            walk_statement_for_value_rewrite(&s.body, lookup, shadowed, edits, bail);
        }
        Statement::FunctionDeclaration(fd) => {
            // Function decls inside callback bodies — walk with their
            // params pushed onto the shadow stack.
            if let Some(body) = &fd.body {
                let before = shadowed.len();
                for param in &fd.params.items {
                    collect_pattern_idents(&param.pattern, shadowed);
                }
                for stmt in &body.statements {
                    walk_statement_for_value_rewrite(stmt, lookup, shadowed, edits, bail);
                }
                shadowed.truncate(before);
            }
        }
        Statement::ThrowStatement(t) => {
            walk_value_expr(&t.argument, lookup, shadowed, edits, bail, false);
        }
        _ => {}
    }
}

/// Collect leaf binding-identifier names from a pattern into
/// `shadowed`. Used for arrow/function param scoping.
fn collect_pattern_idents(
    pat: &oxc_ast::ast::BindingPattern<'_>,
    shadowed: &mut Vec<String>,
) {
    use oxc_ast::ast::BindingPattern;
    match pat {
        BindingPattern::BindingIdentifier(id) => {
            shadowed.push(id.name.to_string());
        }
        BindingPattern::ObjectPattern(op) => {
            for prop in &op.properties {
                collect_pattern_idents(&prop.value, shadowed);
            }
            if let Some(rest) = &op.rest {
                collect_pattern_idents(&rest.argument, shadowed);
            }
        }
        BindingPattern::ArrayPattern(ap) => {
            for elem in ap.elements.iter().flatten() {
                collect_pattern_idents(elem, shadowed);
            }
            if let Some(rest) = &ap.rest {
                collect_pattern_idents(&rest.argument, shadowed);
            }
        }
        BindingPattern::AssignmentPattern(asn) => {
            collect_pattern_idents(&asn.left, shadowed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup_fn(name: &str) -> Option<Option<String>> {
        match name {
            "item" => Some(Some(
                "(typeof items) extends Iterable<infer T> ? T : never".to_string(),
            )),
            "shadowed_unresolvable" => Some(None),
            _ => None,
        }
    }

    #[test]
    fn bare_identifier() {
        let out = rewrite_slot_attr_expr("item", &lookup_fn).unwrap();
        assert_eq!(
            out,
            "((typeof items) extends Iterable<infer T> ? T : never)"
        );
    }

    #[test]
    fn member_expression() {
        let out = rewrite_slot_attr_expr("item.foo", &lookup_fn).unwrap();
        assert_eq!(
            out,
            "((typeof items) extends Iterable<infer T> ? T : never)[\"foo\"]"
        );
    }

    #[test]
    fn nested_member_expression() {
        let out = rewrite_slot_attr_expr("item.foo.bar", &lookup_fn).unwrap();
        assert_eq!(
            out,
            "((typeof items) extends Iterable<infer T> ? T : never)[\"foo\"][\"bar\"]"
        );
    }

    #[test]
    fn computed_member_with_literal_key() {
        let out = rewrite_slot_attr_expr("item['foo']", &lookup_fn).unwrap();
        assert_eq!(
            out,
            "((typeof items) extends Iterable<infer T> ? T : never)[\"foo\"]"
        );
    }

    #[test]
    fn module_identifier_returns_none() {
        // Caller splices source verbatim for module-scope refs.
        assert_eq!(rewrite_slot_attr_expr("modScope", &lookup_fn), None);
    }

    #[test]
    fn shadowed_unresolvable_bails() {
        assert_eq!(
            rewrite_slot_attr_expr("shadowed_unresolvable", &lookup_fn),
            None
        );
        assert_eq!(
            rewrite_slot_attr_expr("shadowed_unresolvable.foo", &lookup_fn),
            None
        );
    }

    #[test]
    fn unsupported_shapes_return_none() {
        // Function call, ternary, binary, object, optional chain,
        // computed-non-literal-key — all bail.
        assert_eq!(rewrite_slot_attr_expr("item.foo()", &lookup_fn), None);
        assert_eq!(rewrite_slot_attr_expr("item ?? other", &lookup_fn), None);
        assert_eq!(rewrite_slot_attr_expr("item?.foo", &lookup_fn), None);
        assert_eq!(rewrite_slot_attr_expr("item[0]", &lookup_fn), None);
        assert_eq!(rewrite_slot_attr_expr("{ x: item }", &lookup_fn), None);
    }

    // Round-7 follow-up #1 walker tests.

    fn r7_lookup(name: &str) -> Option<Option<String>> {
        match name {
            "item" => Some(Some("ItemTy".to_string())),
            "row" => Some(Some("RowTy".to_string())),
            "drop" => Some(None),
            _ => None,
        }
    }

    #[test]
    fn value_walker_no_shadow_returns_no_edits() {
        assert_eq!(
            rewrite_slot_attr_expr_value("modScope", &r7_lookup),
            ValueRewrite::NoEdits
        );
        assert_eq!(
            rewrite_slot_attr_expr_value("foo(bar)", &r7_lookup),
            ValueRewrite::NoEdits
        );
    }

    #[test]
    fn value_walker_call_with_shadowed_arg() {
        let ValueRewrite::Rewritten(out) =
            rewrite_slot_attr_expr_value("foo(item)", &r7_lookup)
        else {
            panic!("expected rewrite");
        };
        assert_eq!(out, "foo((undefined as any as (ItemTy)))");
    }

    #[test]
    fn value_walker_object_shorthand_expands() {
        let ValueRewrite::Rewritten(out) =
            rewrite_slot_attr_expr_value("{ item }", &r7_lookup)
        else {
            panic!("expected rewrite");
        };
        assert_eq!(out, "{ item: (undefined as any as (ItemTy)) }");
    }

    #[test]
    fn value_walker_object_key_skipped() {
        // `item` is the value here, not the key — gets rewritten.
        let ValueRewrite::Rewritten(out) =
            rewrite_slot_attr_expr_value("{ k: item }", &r7_lookup)
        else {
            panic!("expected rewrite");
        };
        assert_eq!(out, "{ k: (undefined as any as (ItemTy)) }");
    }

    #[test]
    fn value_walker_logical_or() {
        let ValueRewrite::Rewritten(out) =
            rewrite_slot_attr_expr_value("fallback ?? item", &r7_lookup)
        else {
            panic!("expected rewrite");
        };
        assert_eq!(out, "fallback ?? (undefined as any as (ItemTy))");
    }

    #[test]
    fn value_walker_member_property_skipped() {
        // `foo` in `item.foo` is a property NAME, not an identifier
        // reference — must NOT be rewritten. `item` IS rewritten.
        let ValueRewrite::Rewritten(out) =
            rewrite_slot_attr_expr_value("item.foo", &r7_lookup)
        else {
            panic!("expected rewrite");
        };
        assert_eq!(out, "(undefined as any as (ItemTy)).foo");
    }

    #[test]
    fn value_walker_bails_on_unresolvable() {
        assert_eq!(
            rewrite_slot_attr_expr_value("foo(drop)", &r7_lookup),
            ValueRewrite::Bailed
        );
        assert_eq!(
            rewrite_slot_attr_expr_value("{ drop }", &r7_lookup),
            ValueRewrite::Bailed
        );
    }

    #[test]
    fn value_walker_arrow_inner_left_alone() {
        // Arrow body's `item` could refer to the lambda parameter
        // (same name) — bailing out of the body is conservative.
        // `items.map(item => item.x)` — the OUTER `items` (if shadowed)
        // would be rewritten; the inner arrow body is left alone.
        let ValueRewrite::Rewritten(out) =
            rewrite_slot_attr_expr_value("row.foo(item => item.x)", &r7_lookup)
        else {
            panic!("expected rewrite");
        };
        // `row` rewritten; arrow body untouched.
        assert!(out.starts_with("(undefined as any as (RowTy))"));
        assert!(out.contains("item => item.x"));
    }

    #[test]
    fn value_walker_two_inner_idents() {
        let ValueRewrite::Rewritten(out) =
            rewrite_slot_attr_expr_value("foo(item, row)", &r7_lookup)
        else {
            panic!("expected rewrite");
        };
        assert_eq!(
            out,
            "foo((undefined as any as (ItemTy)), (undefined as any as (RowTy)))"
        );
    }
}
