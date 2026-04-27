//! Rewrite `let X: Type = $state(null | undefined)` declarations to include
//! an explicit generic: `let X: Type = $state<Type>(null | undefined)`.
//!
//! Context. Our shim previously declared
//!
//! ```ts
//! declare function $state<T>(initial: null): T;
//! declare function $state<T>(initial: undefined): T;
//! declare function $state<T>(initial: T): T;
//! declare function $state<T>(): T | undefined;
//! ```
//!
//! The two literal-type overloads were there to preserve `T` against
//! the assignment context when the initializer was a bare `null` /
//! `undefined` (the bind:this pattern). Without them, the single-T
//! overload binds T to `null` from the argument, which CFA then
//! narrows to `never` inside `if (el) { ... }`, firing TS2339
//! "Property X does not exist on type 'never'".
//!
//! But having those literal-type overloads trips a separate tsgo
//! inference bug: `$state<Promise<T>>(new Promise(() => {}))` fails
//! with TS2769 because the presence of the literal overloads
//! disables contextual-type propagation from the explicit `<T>`
//! through to the argument. `new Promise(() => {})` widens to
//! `Promise<unknown>`, which doesn't fit `Promise<T>`.
//!
//! Injecting the explicit generic from the variable's annotation
//! lets us drop the literal-type overloads entirely. The
//! bind:this pattern now works because `T` comes from the inserted
//! `<Type>`, and the Promise pattern works because the remaining
//! single `<T>(initial: T): T` overload lets tsgo propagate
//! contextual typing without interference.
//!
//! Scope:
//! - Only top-level `VariableDeclaration` statements.
//! - Only `let X: Type = $state(null | undefined)` shape — one
//!   declarator per call-site match, with a type annotation AND a
//!   nullish-literal initializer AND no existing type parameters on
//!   the call.
//! - Multi-declarator lists (`let a: A = $state(null), b: B = ...`)
//!   are handled — each declarator is checked independently.
//! - Insertions are applied in reverse byte order so earlier positions
//!   aren't shifted by later insertions.

use oxc_allocator::Allocator;
use oxc_ast::ast::{Argument, Expression, Statement, VariableDeclarator};
use oxc_span::GetSpan;
use svn_parser::{ScriptLang, parse_script_body};

/// Walk top-level `let X: Type = $state(null | undefined)` declarations
/// and return a new body string with explicit generics injected. When
/// no such declaration is found, returns the input unchanged
/// (cheap early-out via `to_string` clone — the common case on
/// components without bind:this).
pub fn rewrite(content: &str, lang: ScriptLang) -> String {
    let alloc = Allocator::default();
    let parsed = parse_script_body(&alloc, content, lang);

    let mut insertions: Vec<(usize, String)> = Vec::new();
    for stmt in &parsed.program.body {
        let Statement::VariableDeclaration(decl) = stmt else {
            continue;
        };
        for declarator in &decl.declarations {
            if let Some(ins) = detect_site(declarator, content) {
                insertions.push(ins);
            }
        }
    }

    if insertions.is_empty() {
        return content.to_string();
    }

    // Reverse-sort by position so later insertions don't shift earlier
    // ones. `insert_str` at a lower index would move everything above it.
    insertions.sort_by_key(|(pos, _)| std::cmp::Reverse(*pos));

    let mut out = content.to_string();
    for (pos, text) in insertions {
        out.insert_str(pos, &text);
    }
    out
}

/// If `declarator` is `let NAME: TYPE = $state(null | undefined)` with no
/// existing type arguments on the call, return the byte position at which
/// to splice `<TYPE>` and the text to splice.
fn detect_site(declarator: &VariableDeclarator<'_>, source: &str) -> Option<(usize, String)> {
    // Binding must carry a type annotation — that's where we pull the
    // explicit generic from.
    let type_anno = declarator.type_annotation.as_ref()?;
    let type_span = type_anno.type_annotation.span();
    let type_text = source.get(type_span.start as usize..type_span.end as usize)?;

    // Initializer must be a `$state(...)` call.
    let init = declarator.init.as_ref()?;
    let Expression::CallExpression(call) = init else {
        return None;
    };
    let Expression::Identifier(callee_id) = &call.callee else {
        return None;
    };
    if callee_id.name != "$state" {
        return None;
    }

    // Don't double-specify: if the call already has explicit type
    // parameters, leave it alone. User wrote `$state<Foo>(...)` on
    // purpose.
    if call.type_arguments.is_some() {
        return None;
    }

    // Exactly one argument: a nullish literal. `undefined` parses as an
    // identifier reference, not a literal — match both forms.
    if call.arguments.len() != 1 {
        return None;
    }
    let is_nullish = match call.arguments.first()? {
        Argument::NullLiteral(_) => true,
        Argument::Identifier(id) => id.name == "undefined",
        _ => false,
    };
    if !is_nullish {
        return None;
    }

    // Splice after `$state`'s identifier span, before the `(`.
    let insert_at = callee_id.span.end as usize;
    let insertion = format!("<{type_text}>");
    Some((insert_at, insertion))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(src: &str) -> String {
        rewrite(src, ScriptLang::Ts)
    }

    #[test]
    fn rewrites_typed_null_state() {
        let src = "let el: HTMLInputElement | null = $state(null);";
        assert_eq!(
            ts(src),
            "let el: HTMLInputElement | null = $state<HTMLInputElement | null>(null);"
        );
    }

    #[test]
    fn rewrites_typed_undefined_state() {
        let src = "let el: string | undefined = $state(undefined);";
        assert_eq!(
            ts(src),
            "let el: string | undefined = $state<string | undefined>(undefined);"
        );
    }

    #[test]
    fn leaves_untyped_alone() {
        // No annotation → let tsgo infer from the argument literally.
        // `let x = $state(null)` → x: null is what the user asked for.
        let src = "let x = $state(null);";
        assert_eq!(ts(src), src);
    }

    #[test]
    fn leaves_explicit_generic_alone() {
        let src = "let el: Foo | null = $state<Bar>(null);";
        assert_eq!(ts(src), src);
    }

    #[test]
    fn leaves_non_state_call_alone() {
        let src = "let el: Foo = someOtherFn(null);";
        assert_eq!(ts(src), src);
    }

    #[test]
    fn leaves_non_nullish_arg_alone() {
        // Argument is a constructor call, not null/undefined. The main
        // `$state<T>(initial: T): T` overload handles it via context.
        let src = "let p: Promise<number> = $state(Promise.resolve(1));";
        assert_eq!(ts(src), src);
    }

    #[test]
    fn rewrites_every_match_in_multi_declarator() {
        let src = "let a: A | null = $state(null), b: B | undefined = $state(undefined);";
        assert_eq!(
            ts(src),
            "let a: A | null = $state<A | null>(null), b: B | undefined = $state<B | undefined>(undefined);"
        );
    }

    #[test]
    fn leaves_non_toplevel_state_alone() {
        // Nested `$state(null)` call inside another expression — we only
        // rewrite the simple shape.
        let src = "let x: Foo = outer($state(null));";
        assert_eq!(ts(src), src);
    }

    #[test]
    fn preserves_surrounding_whitespace_and_comments() {
        let src = "\
// Bind:this target
let inputEl: HTMLInputElement | null = $state(null) // initialized lazily
";
        let got = ts(src);
        assert!(got.contains("$state<HTMLInputElement | null>(null)"));
        assert!(got.contains("// Bind:this target"));
        assert!(got.contains("// initialized lazily"));
    }

    #[test]
    fn preserves_complex_type_annotation() {
        let src = "let x: Map<string, Promise<Foo[]> | undefined> = $state(null);";
        let expected = "let x: Map<string, Promise<Foo[]> | undefined> = $state<Map<string, Promise<Foo[]> | undefined>>(null);";
        assert_eq!(ts(src), expected);
    }

    #[test]
    fn handles_const_declarator() {
        // `const` declarators have the same shape; rewrite still applies.
        let src = "const snapshot: Data | null = $state(null);";
        assert_eq!(
            ts(src),
            "const snapshot: Data | null = $state<Data | null>(null);"
        );
    }
}
