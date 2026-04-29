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
    lookup: impl Fn(&str) -> Option<Option<String>>,
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
    rewrite_expression(inner, &lookup)
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
        let out = rewrite_slot_attr_expr("item", lookup_fn).unwrap();
        assert_eq!(
            out,
            "((typeof items) extends Iterable<infer T> ? T : never)"
        );
    }

    #[test]
    fn member_expression() {
        let out = rewrite_slot_attr_expr("item.foo", lookup_fn).unwrap();
        assert_eq!(
            out,
            "((typeof items) extends Iterable<infer T> ? T : never)[\"foo\"]"
        );
    }

    #[test]
    fn nested_member_expression() {
        let out = rewrite_slot_attr_expr("item.foo.bar", lookup_fn).unwrap();
        assert_eq!(
            out,
            "((typeof items) extends Iterable<infer T> ? T : never)[\"foo\"][\"bar\"]"
        );
    }

    #[test]
    fn computed_member_with_literal_key() {
        let out = rewrite_slot_attr_expr("item['foo']", lookup_fn).unwrap();
        assert_eq!(
            out,
            "((typeof items) extends Iterable<infer T> ? T : never)[\"foo\"]"
        );
    }

    #[test]
    fn module_identifier_returns_none() {
        // Caller splices source verbatim for module-scope refs.
        assert_eq!(rewrite_slot_attr_expr("modScope", lookup_fn), None);
    }

    #[test]
    fn shadowed_unresolvable_bails() {
        assert_eq!(
            rewrite_slot_attr_expr("shadowed_unresolvable", lookup_fn),
            None
        );
        assert_eq!(
            rewrite_slot_attr_expr("shadowed_unresolvable.foo", lookup_fn),
            None
        );
    }

    #[test]
    fn unsupported_shapes_return_none() {
        // Function call, ternary, binary, object, optional chain,
        // computed-non-literal-key — all bail.
        assert_eq!(rewrite_slot_attr_expr("item.foo()", lookup_fn), None);
        assert_eq!(rewrite_slot_attr_expr("item ?? other", lookup_fn), None);
        assert_eq!(rewrite_slot_attr_expr("item?.foo", lookup_fn), None);
        assert_eq!(rewrite_slot_attr_expr("item[0]", lookup_fn), None);
        assert_eq!(rewrite_slot_attr_expr("{ x: item }", lookup_fn), None);
    }
}
