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

use oxc_ast::ast::{
    BindingPatternKind, BindingProperty, Declaration, Expression, ModuleExportName, PropertyKey,
    Statement, VariableDeclaration,
};
use oxc_span::GetSpan;
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

/// Find the *type source text* for the component's Props bag — either
/// the explicit `$props()` type annotation (Svelte 5) or a synthesized
/// object type built from Svelte 4-style `export let` declarations.
///
/// Priority order (first match wins):
///
/// 1. `let { ... }: PropType = $props()` — return PropType's source.
/// 2. `let { ... } = $props<PropType>()` — return PropType's source.
/// 3. `export let foo: T; export let bar = 42;` (Svelte 4 style) —
///    synthesize `{ foo: T; bar?: any; ... }` from each top-level
///    `export let`/`export const` declaration.
/// 4. None of the above → `None`. Callers treat `None` as "no typed
///    prop shape available; emit default as `any`".
///
/// Returned shapes 1-2 are a slice of `source` cloned into a `String`
/// (trusting the user's original syntax verbatim). Shape 3 is freshly
/// synthesized from the declarator name + optional type annotation;
/// declarations without an explicit type fall back to `any`.
///
/// `export const foo: T = ...` is handled identically to `export let`
/// here — upstream treats `export const` as a read-only prop (a
/// getter); for the purposes of the default export's Props shape the
/// distinction is irrelevant. Both contribute a property to the
/// synthesized object type.
pub fn find_props_type_source(program: &oxc_ast::ast::Program<'_>, source: &str) -> Option<String> {
    // Shape 1 / Shape 2: prefer an explicit `$props()` annotation when
    // present. Svelte 5 components win over any accidental stray
    // `export let` in the body (which would be a user error anyway —
    // Svelte 5 doesn't use `export let` for props).
    for stmt in &program.body {
        let Statement::VariableDeclaration(decl) = stmt else {
            continue;
        };
        for declarator in &decl.declarations {
            let Some(init) = declarator.init.as_ref() else {
                continue;
            };
            if !is_props_call_like(init) {
                continue;
            }
            if let Some(ty) = declarator.id.type_annotation.as_ref() {
                let span = ty.type_annotation.span();
                if let Some(slice) = source.get(span.start as usize..span.end as usize) {
                    return Some(slice.to_string());
                }
            }
            if let Expression::CallExpression(call) = init {
                if let Some(tp) = call.type_parameters.as_ref() {
                    if let Some(arg) = tp.params.first() {
                        let span = arg.span();
                        if let Some(slice) = source.get(span.start as usize..span.end as usize) {
                            return Some(slice.to_string());
                        }
                    }
                }
            }
        }
    }
    // SVELTE-4-COMPAT: `interface $$Props { … }` at module scope is
    // the pre-Svelte-5 convention for declaring component props. When
    // no `$props()` call was found above, use `$$Props` as the Props
    // type source. The interface declaration itself gets hoisted by
    // script_split alongside other user interfaces, so module-scope
    // consumers of the emitted `Component<$$Props>` can resolve it.
    for stmt in &program.body {
        if let Statement::TSInterfaceDeclaration(iface) = stmt {
            if iface.id.name == "$$Props" {
                return Some("$$Props".to_string());
            }
        }
    }
    // Shape 3: Svelte 4 fallback. Walk top-level `export let` / `export
    // const` declarations and synthesize an inline object type. Only
    // runs when no `$props()` call was seen above.
    synthesize_props_type_from_export_let(program, source)
}

/// Build an inline object type literal from top-level `export let` /
/// `export const` declarations in an instance script. Returns `None`
/// when there are no such declarations.
///
/// `program` here is the instance script AST. Module-script `export
/// let`s are NOT component props — those are module-scope exports.
/// Callers must pass the instance-script program, not the module
/// script's.
///
/// Each declarator becomes one property on the synthesized type:
///
/// - `export let foo: T;` → `foo: T;` (required)
/// - `export let foo: T = v;` → `foo?: T;` (optional — has default)
/// - `export let foo = v;` → `foo?: any;` (no type; inference is
///   nontrivial without tsgo, so we pick `any` over mislabeling)
/// - `export let foo;` → `foo: any;` (no type, no default)
/// - `export const foo: T = v;` → `foo?: T;` (has initializer; `const`
///   always has one, so always optional for defaulting)
///
/// Non-identifier patterns (e.g. `export let { a } = obj;`) are skipped:
/// destructured exports are not valid Svelte prop declarations.
fn synthesize_props_type_from_export_let(
    program: &oxc_ast::ast::Program<'_>,
    source: &str,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for stmt in &program.body {
        let Statement::ExportNamedDeclaration(export_decl) = stmt else {
            continue;
        };
        if let Some(Declaration::VariableDeclaration(var_decl)) = &export_decl.declaration {
            append_props_from_var_decl(var_decl, source, &mut parts);
        }
        // `export { name as alias, ... }` specifier form. Svelte 4
        // components use this to expose a local under a JS reserved
        // name, most commonly `export { className as class }` so
        // consumers can pass `class={...}` without hitting `class`
        // being a keyword in the source. Each specifier contributes
        // one prop; the local's declared type (if any) is preserved.
        for spec in &export_decl.specifiers {
            append_prop_from_export_specifier(spec, program, source, &mut parts);
        }
    }
    if parts.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(parts.iter().map(|p| p.len() + 2).sum::<usize>() + 4);
    out.push_str("{ ");
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(part);
    }
    out.push_str(" }");
    Some(out)
}

/// Extract a single property from `export { local as alias }`. Looks up
/// `local` in the program's top-level `let`/`const`/`var` declarations
/// to pick up its type annotation and initializer (which decides the
/// optional marker). When `local` can't be found or has no annotation,
/// the prop is typed `any`. The alias becomes the public key so
/// consumers write `<Foo {alias}=...>`.
fn append_prop_from_export_specifier(
    spec: &oxc_ast::ast::ExportSpecifier<'_>,
    program: &oxc_ast::ast::Program<'_>,
    source: &str,
    out: &mut Vec<String>,
) {
    // Re-export `export { X } from 'mod'` (source specifier on the
    // parent statement) and namespace re-exports don't declare a
    // component prop — the parent's `source` field catches re-exports;
    // here we only care about local-to-alias renames.
    let Some(alias) = module_export_name_str(&spec.exported) else {
        return;
    };
    let Some(local) = module_export_name_str(&spec.local) else {
        return;
    };
    let (ty_text, has_init) = find_local_type_and_init(program, source, local);
    let optional_marker = if has_init { "?" } else { "" };
    let ty_src = ty_text.unwrap_or("any");
    out.push(format!("{alias}{optional_marker}: {ty_src};"));
}

/// Readable `str` from a `ModuleExportName` variant. Returns `None` for
/// `StringLiteral` — a string-literal alias like `export { foo as 'a-b' }`
/// isn't usable as an object-type key here (we'd have to quote it, and
/// the Svelte idiom doesn't produce them).
fn module_export_name_str<'a>(name: &'a ModuleExportName<'_>) -> Option<&'a str> {
    match name {
        ModuleExportName::IdentifierName(id) => Some(id.name.as_str()),
        ModuleExportName::IdentifierReference(id) => Some(id.name.as_str()),
        ModuleExportName::StringLiteral(_) => None,
    }
}

/// Walk the program's top-level `let`/`const`/`var` declarations looking
/// for one that binds `name`. Returns `(type_text, has_init)` — the type
/// annotation's source slice when present, and whether the declarator
/// has an initializer (so the caller can mark the prop optional vs
/// required the same way `append_props_from_var_decl` does for the
/// `export let` form).
fn find_local_type_and_init<'s>(
    program: &oxc_ast::ast::Program<'_>,
    source: &'s str,
    name: &str,
) -> (Option<&'s str>, bool) {
    for stmt in &program.body {
        let Statement::VariableDeclaration(decl) = stmt else {
            continue;
        };
        for declarator in &decl.declarations {
            let BindingPatternKind::BindingIdentifier(id) = &declarator.id.kind else {
                continue;
            };
            if id.name.as_str() != name {
                continue;
            }
            let ty_text = declarator
                .id
                .type_annotation
                .as_ref()
                .map(|ty| ty.type_annotation.span())
                .and_then(|span| source.get(span.start as usize..span.end as usize));
            return (ty_text, declarator.init.is_some());
        }
    }
    (None, false)
}

fn append_props_from_var_decl(
    var_decl: &VariableDeclaration<'_>,
    source: &str,
    out: &mut Vec<String>,
) {
    for declarator in &var_decl.declarations {
        let BindingPatternKind::BindingIdentifier(id) = &declarator.id.kind else {
            // Destructured exports aren't valid Svelte prop declarations;
            // skip them rather than invent a synthetic name.
            continue;
        };
        let name = id.name.as_str();
        let has_init = declarator.init.is_some();
        let ty_text: Option<&str> = declarator
            .id
            .type_annotation
            .as_ref()
            .map(|ty| ty.type_annotation.span())
            .and_then(|span| source.get(span.start as usize..span.end as usize));
        // An initializer makes the prop optional (caller can omit and
        // the default kicks in). No initializer + no type → required
        // `any`. No initializer + type → required of that type.
        let optional_marker = if has_init { "?" } else { "" };
        let ty_src = ty_text.unwrap_or("any");
        out.push(format!("{name}{optional_marker}: {ty_src};"));
    }
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

    fn props_type(src: &str) -> Option<String> {
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, src, ScriptLang::Ts);
        find_props_type_source(&parsed.program, src)
    }

    #[test]
    fn props_type_from_destructure_annotation() {
        let src = "let { foo }: { foo: string } = $props();";
        assert_eq!(props_type(src).as_deref(), Some("{ foo: string }"));
    }

    #[test]
    fn props_type_from_generic_arg() {
        let src = "let { foo } = $props<{ foo: string }>();";
        assert_eq!(props_type(src).as_deref(), Some("{ foo: string }"));
    }

    #[test]
    fn props_type_none_for_untyped_props_call() {
        assert_eq!(props_type("let { foo } = $props();"), None);
    }

    #[test]
    fn synth_props_type_from_single_export_let_with_type() {
        let src = "export let b: (v: string) => void;";
        assert_eq!(
            props_type(src).as_deref(),
            Some("{ b: (v: string) => void; }")
        );
    }

    #[test]
    fn synth_props_type_makes_default_initialized_optional() {
        let src = "export let count: number = 0;";
        assert_eq!(props_type(src).as_deref(), Some("{ count?: number; }"));
    }

    #[test]
    fn synth_props_type_no_type_no_default_is_any_required() {
        let src = "export let data;";
        assert_eq!(props_type(src).as_deref(), Some("{ data: any; }"));
    }

    #[test]
    fn synth_props_type_no_type_with_default_is_any_optional() {
        let src = "export let count = 42;";
        assert_eq!(props_type(src).as_deref(), Some("{ count?: any; }"));
    }

    #[test]
    fn synth_props_type_multiple_export_lets() {
        let src = "export let a: string;\nexport let b: number = 1;";
        assert_eq!(
            props_type(src).as_deref(),
            Some("{ a: string; b?: number; }")
        );
    }

    #[test]
    fn synth_props_type_treats_export_const_like_export_let() {
        // `export const foo: T = v` → read-only prop; still contributes
        // a (optional) property to the synthesized type.
        let src = "export const foo: string = 'x';";
        assert_eq!(props_type(src).as_deref(), Some("{ foo?: string; }"));
    }

    #[test]
    fn props_call_wins_over_export_let() {
        // A script with BOTH `$props()` (annotated) and a stray `export
        // let` should prefer the explicit `$props()` annotation.
        let src = "export let stray: number;\nlet { foo }: { foo: string } = $props();";
        assert_eq!(props_type(src).as_deref(), Some("{ foo: string }"));
    }

    #[test]
    fn export_fn_does_not_contribute_props() {
        // `export function` is an exported helper, not a prop; ignored.
        let src = "export function helper() {}";
        assert_eq!(props_type(src), None);
    }

    #[test]
    fn export_type_alias_does_not_contribute_props() {
        let src = "export type Foo = number;";
        assert_eq!(props_type(src), None);
    }

    #[test]
    fn dollar_dollar_props_fallback_when_no_props_call() {
        // Svelte-4 `interface $$Props { ... }` convention. With no
        // `$props()` call, $$Props is the Props type source.
        let src = "interface $$Props { foo: number }";
        assert_eq!(props_type(src).as_deref(), Some("$$Props"));
    }

    #[test]
    fn props_call_wins_over_dollar_dollar_props() {
        // If both are present, the explicit `$props()` annotation wins.
        let src = "interface $$Props { foo: number }\nlet { bar }: { bar: string } = $props();";
        assert_eq!(props_type(src).as_deref(), Some("{ bar: string }"));
    }

    #[test]
    fn export_let_wins_over_nothing_but_not_over_dollar_dollar_props() {
        // $$Props wins over the export-let fallback (both are Svelte-4
        // conventions, but $$Props is more explicit and newer).
        let src = "interface $$Props { foo: number }\nexport let stray: number;";
        assert_eq!(props_type(src).as_deref(), Some("$$Props"));
    }

    #[test]
    fn export_specifier_rename_contributes_class_prop() {
        // Svelte 4 pattern: rename a local to a JS reserved name so it
        // can be used as a component prop.
        let src = "let className: any = \"\";\nexport { className as class };";
        assert_eq!(props_type(src).as_deref(), Some("{ class?: any; }"));
    }

    #[test]
    fn export_specifier_preserves_local_type() {
        let src = "let n: number = 0;\nexport { n as count };";
        assert_eq!(props_type(src).as_deref(), Some("{ count?: number; }"));
    }

    #[test]
    fn export_specifier_missing_init_marks_required() {
        let src = "let n: number;\nexport { n as count };";
        assert_eq!(props_type(src).as_deref(), Some("{ count: number; }"));
    }

    #[test]
    fn export_specifier_missing_local_falls_back_to_any() {
        // Export of an undeclared local — pathological but don't panic.
        let src = "export { missing as foo };";
        assert_eq!(props_type(src).as_deref(), Some("{ foo: any; }"));
    }

    #[test]
    fn export_specifier_combined_with_export_let() {
        let src =
            "export let width = 40;\nlet className: any = \"\";\nexport { className as class };";
        assert_eq!(
            props_type(src).as_deref(),
            Some("{ width?: any; class?: any; }")
        );
    }
}
