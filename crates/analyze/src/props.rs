//! Props analysis — find `let { ... } = $props()` destructuring patterns.
//!
//! For each prop declared via Svelte 5's `$props()` rune, record the local
//! name bound by the destructuring. Emit consumes this list to produce
//! `void <local_name>;` references: destructured props are part of the
//! component's public API and must be treated as "used" even when the
//! component body doesn't touch them (e.g., props only consumed via
//! `bind:`, `<svelte:element {...}>`, or by a subcomponent after spread).
//!
//! Without per-prop void-refs, `noUnusedLocals` flags every destructured
//! prop as unused — roughly 80 % of a typical project's error budget comes from
//! this one gap.
//!
//! ### Destructuring patterns handled
//!
//! - `let { foo } = $props()`                          → local = `foo`
//! - `let { foo = defaultVal } = $props()`             → local = `foo`
//! - `let { class: classValue } = $props()`            → local = `classValue`
//! - `let { foo, ...rest } = $props()`                 → locals = `foo`, `rest`
//! - `let { foo }: FooProps = $props()`                → local = `foo`
//! - `let { foo } = $props<Props>()`                   → local = `foo`
//!
//! Nested destructuring (`let { foo: { bar } } = $props()`) is walked
//! recursively; every leaf identifier is recorded.

use oxc_ast::ast::{BindingPatternKind, BindingProperty, Expression, PropertyKey, Statement};
use smol_str::SmolStr;
use svn_core::Range;

/// One destructured prop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropInfo {
    /// The local name introduced by the destructuring — what later code
    /// in the script refers to. For a rename `{ class: classValue }` this
    /// is `classValue`, not `class`.
    pub local_name: SmolStr,
    /// Byte range of the local identifier in the source.
    pub range: Range,
    /// True for `...rest` elements.
    pub is_rest: bool,
}

/// Find every `let { ... } = $props()` destructuring in `program` and
/// return the local names introduced. Order is source order.
pub fn find_props(program: &oxc_ast::ast::Program<'_>) -> Vec<PropInfo> {
    let mut out = Vec::new();
    // Only top-level: $props() calls elsewhere are not component-level
    // prop declarations.
    for stmt in &program.body {
        if let Statement::VariableDeclaration(decl) = stmt {
            for declarator in &decl.declarations {
                if declarator.init.as_ref().is_some_and(is_props_call_like) {
                    collect_from_binding(&declarator.id.kind, &mut out);
                }
            }
        }
    }
    out
}

/// Does this expression look like a call to the `$props` rune?
///
/// Matches `$props()`, `$props<Type>()`, `$props<{...}>()`. Doesn't match
/// dotted variants (`$props.id()` — that's a different rune).
fn is_props_call_like(expr: &Expression<'_>) -> bool {
    let Expression::CallExpression(call) = expr else {
        return false;
    };
    matches!(&call.callee, Expression::Identifier(id) if id.name == "$props")
}

fn collect_from_binding(pat: &BindingPatternKind<'_>, out: &mut Vec<PropInfo>) {
    match pat {
        BindingPatternKind::ObjectPattern(obj) => {
            for prop in &obj.properties {
                collect_from_object_property(prop, out);
            }
            if let Some(rest) = &obj.rest {
                collect_rest(&rest.argument.kind, out, true);
            }
        }
        // `let [a, b, c] = $props()` isn't a valid Svelte pattern ($props
        // returns an object), but be defensive.
        BindingPatternKind::ArrayPattern(arr) => {
            for el in arr.elements.iter().flatten() {
                collect_from_binding(&el.kind, out);
            }
        }
        BindingPatternKind::BindingIdentifier(id) => {
            out.push(PropInfo {
                local_name: SmolStr::from(id.name.as_str()),
                range: Range::new(id.span.start, id.span.end),
                is_rest: false,
            });
        }
        BindingPatternKind::AssignmentPattern(asn) => {
            collect_from_binding(&asn.left.kind, out);
        }
    }
}

fn collect_from_object_property(prop: &BindingProperty<'_>, out: &mut Vec<PropInfo>) {
    // Shorthand `{ foo }` vs rename `{ foo: bar }` — both come through
    // `value` on the property. For shorthand the key and value identifier
    // are the same; for rename they differ. Either way, the *local* name
    // is in `value` — which is what we record.
    let _ = prop.key; // intentionally unused — local name lives in value
    if let PropertyKey::StaticIdentifier(_) = &prop.key {
        // nothing needed — value is the binding pattern we care about
    }
    collect_from_binding(&prop.value.kind, out);
}

fn collect_rest(pat: &BindingPatternKind<'_>, out: &mut Vec<PropInfo>, is_rest: bool) {
    match pat {
        BindingPatternKind::BindingIdentifier(id) => {
            out.push(PropInfo {
                local_name: SmolStr::from(id.name.as_str()),
                range: Range::new(id.span.start, id.span.end),
                is_rest,
            });
        }
        // Rest patterns holding further destructuring are allowed but
        // unusual; walk recursively.
        other => collect_from_binding(other, out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use svn_parser::{ScriptLang, parse_script_body};

    fn props(src: &str) -> Vec<String> {
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, src, ScriptLang::Ts);
        find_props(&parsed.program)
            .into_iter()
            .map(|p| p.local_name.to_string())
            .collect()
    }

    #[test]
    fn empty_script_returns_empty() {
        assert!(props("").is_empty());
    }

    #[test]
    fn no_props_call_returns_empty() {
        assert!(props("const x = 1;").is_empty());
    }

    #[test]
    fn simple_shorthand_prop() {
        assert_eq!(props("let { foo } = $props();"), vec!["foo"]);
    }

    #[test]
    fn multiple_shorthand_props() {
        assert_eq!(props("let { a, b, c } = $props();"), vec!["a", "b", "c"]);
    }

    #[test]
    fn prop_with_default() {
        assert_eq!(props("let { foo = 'bar' } = $props();"), vec!["foo"]);
    }

    #[test]
    fn renamed_prop_returns_local_name() {
        // `{ class: classValue }` — local binding is `classValue`, NOT `class`.
        // Using `class` would produce `void class;` which is a JS reserved-
        // word error. The local_name is what we record.
        assert_eq!(
            props("let { class: classValue } = $props();"),
            vec!["classValue"]
        );
    }

    #[test]
    fn renamed_with_default() {
        assert_eq!(
            props("let { class: classValue = 'default' } = $props();"),
            vec!["classValue"]
        );
    }

    #[test]
    fn rest_prop() {
        let src = "let { foo, ...rest } = $props();";
        assert_eq!(props(src), vec!["foo", "rest"]);
    }

    #[test]
    fn rest_is_flagged_on_info() {
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, "let { a, ...rest } = $props();", ScriptLang::Ts);
        let info = find_props(&parsed.program);
        assert_eq!(info.len(), 2);
        assert!(!info[0].is_rest);
        assert!(info[1].is_rest);
    }

    #[test]
    fn typed_destructuring() {
        assert_eq!(
            props("let { foo, bar }: { foo: string; bar: number } = $props();"),
            vec!["foo", "bar"]
        );
    }

    #[test]
    fn generic_props_call() {
        assert_eq!(
            props("let { foo } = $props<{ foo: string }>();"),
            vec!["foo"]
        );
    }

    #[test]
    fn props_dot_id_not_recognized_as_props_call() {
        // $props.id() is a different rune; `foo` there isn't a component prop.
        assert!(props("let foo = $props.id();").is_empty());
    }

    #[test]
    fn props_not_at_top_level_ignored() {
        // $props() inside a function isn't valid Svelte; don't extract.
        let src = "function f() { let { foo } = $props(); }";
        assert!(props(src).is_empty());
    }

    #[test]
    fn comment_between_destructured_props() {
        let src = "let {\n  a,\n  /* b comment */\n  b,\n  // c comment\n  c,\n} = $props();";
        assert_eq!(props(src), vec!["a", "b", "c"]);
    }

    #[test]
    fn generics_in_bindable_default() {
        // $bindable<Record<string, number>>({}) — generic args with commas
        // inside < > which trips character-level parsers but not oxc.
        let src = "let { members = $bindable<Record<string, number>>({}), count = 0 } = $props();";
        assert_eq!(props(src), vec!["members", "count"]);
    }

    #[test]
    fn prop_name_with_dollar_suffix() {
        assert_eq!(props("let { parent$ } = $props();"), vec!["parent$"]);
    }

    #[test]
    fn nested_destructuring_recurses() {
        // let { outer: { inner } } = $props() — inner is a leaf binding.
        let src = "let { outer: { inner } } = $props();";
        assert_eq!(props(src), vec!["inner"]);
    }

    #[test]
    fn ranges_point_at_local_identifier() {
        let src = "let { foo } = $props();";
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, src, ScriptLang::Ts);
        let info = find_props(&parsed.program);
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].range.slice(src), "foo");
    }
}
