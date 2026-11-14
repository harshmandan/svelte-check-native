//! Inputs where the port once answered differently from acorn 8.19.0
//! or acorn-typescript 1.0.13. What each asserts is what those answer.

use svn_acorn::{Language, Reading, parse, parse_expression_at};

fn error_of<T>(result: Result<T, svn_acorn::SyntaxError>) -> Option<(String, u32)> {
    result.err().map(|e| (e.message, e.pos))
}

fn expression(code: &str) -> Option<(String, u32)> {
    error_of(parse_expression_at(code, 0, Language::TypeScript))
}

fn module(code: &str, language: Language) -> Option<(String, u32)> {
    error_of(parse(code, Reading::Module, language))
}

/// acorn-typescript asks whether a parenthesized expression can be a
/// parameter without saying it would bind, so `(a.b)` can.
#[test]
fn a_parenthesized_member_expression_passes_for_an_arrow_parameter() {
    let parenthesized_pattern = |at| Some(("Parenthesized pattern".to_string(), at));
    assert_eq!(expression("x ? ((a.b)) : y => z"), parenthesized_pattern(5));
    assert_eq!(expression("((a.b)): void => 1"), parenthesized_pattern(1));
    assert_eq!(expression("f(((a.b)): T => 1)"), parenthesized_pattern(3));
}

#[test]
fn a_decorated_class_expression_starts_at_its_first_decorator() {
    let rvalue = |at| Some(("Assigning to rvalue".to_string(), at));
    for language_reading in [Reading::Module, Reading::Script] {
        let at = |code: &str| error_of(parse(code, language_reading, Language::TypeScript));
        assert_eq!(at("(@dec class {}) = 1"), rvalue(1));
        assert_eq!(at("++@dec class {}"), rvalue(2));
        assert_eq!(at("[@dec class {}] = x"), rvalue(1));
        assert_eq!(at("[@a @b() class {}] = x"), rvalue(1));
    }
}

/// acorn records the keys it has seen as properties of a plain object,
/// where assigning `__proto__` records nothing.
#[test]
fn an_import_attribute_named_proto_is_never_a_duplicate() {
    let js = |code| module(code, Language::JavaScript);
    assert_eq!(
        js(r#"import a from "x" with { __proto__: "a", __proto__: "b" }"#),
        None
    );
    assert_eq!(
        js(r#"import a from "x" with { "__proto__": "a", __proto__: "b" }"#),
        None
    );
    assert_eq!(
        js(r#"import a from "x" with { type: "a", type: "b" }"#),
        Some(("Duplicate attribute key 'type'".to_string(), 36))
    );
}

#[test]
fn import_attribute_keys_with_lone_surrogates_are_compared_by_code_unit() {
    let js = |code| module(code, Language::JavaScript);
    assert_eq!(
        js(r#"import a from "x" with { "\ud800": "a", "\ud801": "b" }"#),
        None
    );
    assert_eq!(
        js(r#"import a from "x" with { "\ud800": "a", "�": "b" }"#),
        None
    );
    assert!(js(r#"import a from "x" with { "\ud800": "a", "\uD800": "b" }"#).is_some());
}

/// A type assertion with nothing to assert leaves acorn-typescript
/// holding the type parameters it read instead, which it returns as
/// the expression.
#[test]
fn bare_type_parameters_are_an_expression() {
    let end = |code| parse_expression_at(code, 0, Language::TypeScript).map_err(|e| e.message);
    assert_eq!(end("<div>}"), Ok(5));
    assert_eq!(end("<T,>}"), Ok(4));
    assert_eq!(end("<a b>}"), Err("Unexpected token".to_string()));
}

/// Private names declared or used inside an attempted reading that is
/// given up are forgotten with it; those of the reading that stands are
/// kept, once.
#[test]
fn private_names_survive_attempted_readings() {
    let ts = |code| module(code, Language::TypeScript);
    assert_eq!(
        ts("class A { #a = 1; m() { const f = <T,>(v: T) => this.#a; return f<number>(1); } }"),
        None
    );
    let undeclared = |at| {
        Some((
            "Private field '#b' must be declared in an enclosing class".to_string(),
            at,
        ))
    };
    assert_eq!(
        ts("class A { #a = 1; m() { return <T,>(v: T) => this.#b; } }"),
        undeclared(50)
    );
    assert_eq!(
        ts("class A { get #a() { return 1 } m() { return (x: number) => x; } set #a(v) {} }"),
        None
    );
    assert_eq!(
        ts("class A { get #a() { return 1 } m = (x) => x; get #a() { return 2 } }"),
        Some(("Identifier '#a' has already been declared".to_string(), 50))
    );
}
