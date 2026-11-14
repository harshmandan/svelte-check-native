//! Input nested or chained far past what anyone writes by hand.
//!
//! acorn recurses on the JavaScript engine's stack and reports running
//! out of it as a syntax error. Where exactly that happens is up to the
//! engine, so these tests pin only what must hold whatever the engine:
//! code well inside acorn's reach is accepted, code far outside it gets
//! acorn's message, and neither brings the process down or runs on
//! without end.

use svn_acorn::{Language, Reading, parse, parse_expression_at};

const OUT_OF_STACK: &str = "Not enough stack space to parse input";

fn script(code: &str, language: Language) -> Result<(), String> {
    parse(code, Reading::Script, language).map_err(|e| e.message)
}

fn out_of_stack(result: Result<(), String>) -> bool {
    result.is_err_and(|message| message == OUT_OF_STACK)
}

fn wrapped(open: &str, inner: &str, close: &str, depth: usize) -> String {
    format!("{}{inner}{}", open.repeat(depth), close.repeat(depth))
}

#[test]
fn nested_types_far_past_the_stack_are_an_error() {
    for (open, close) in [
        ("(", ")"),
        ("Array<", ">"),
        ("[", "]"),
        ("{a:", "}"),
        ("() => ", ""),
        ("A[", "]"),
    ] {
        let code = format!("let x: {};", wrapped(open, "T", close, 50_000));
        assert!(
            out_of_stack(script(&code, Language::TypeScript)),
            "{open}…{close}"
        );
    }
    let code = format!("let x: {}T;", "keyof ".repeat(50_000));
    assert!(out_of_stack(script(&code, Language::TypeScript)));
}

#[test]
fn nested_types_within_reach_are_read() {
    for (open, close) in [("(", ")"), ("Array<", ">"), ("[", "]")] {
        let code = format!("let x: {};", wrapped(open, "T", close, 250));
        assert_eq!(script(&code, Language::TypeScript), Ok(()), "{open}");
    }
    let code = format!("let x: {}T;", "keyof ".repeat(2_000));
    assert_eq!(script(&code, Language::TypeScript), Ok(()));
}

fn both_languages_run_out_of_stack(code: &str) {
    for language in [Language::JavaScript, Language::TypeScript] {
        assert!(out_of_stack(script(code, language)));
    }
}

#[test]
fn nested_array_patterns_far_past_the_stack_are_an_error() {
    both_languages_run_out_of_stack(&format!("let {}", "[".repeat(50_000)));
}

#[test]
fn nested_object_patterns_far_past_the_stack_are_an_error() {
    both_languages_run_out_of_stack(&format!("let {}", "{a:".repeat(50_000)));
}

#[test]
fn a_chain_of_class_heritages_far_past_the_stack_is_an_error() {
    both_languages_run_out_of_stack(&format!("x = {}", "class extends ".repeat(50_000)));
}

#[test]
fn a_chain_of_new_far_past_the_stack_is_an_error() {
    both_languages_run_out_of_stack(&format!("x = {}", "new ".repeat(50_000)));
}

#[test]
fn a_namespace_name_far_past_the_stack_is_an_error() {
    let code = format!("namespace A{} {{}}", ".B".repeat(50_000));
    assert!(out_of_stack(script(&code, Language::TypeScript)));
}

#[test]
fn nested_regular_expression_groups_far_past_the_stack_are_an_error() {
    let code = format!("x = /{}/;", wrapped("(", "a", ")", 50_000));
    assert!(out_of_stack(script(&code, Language::JavaScript)));
    let code = format!("x = /{}/v;", wrapped("[", "a", "]", 50_000));
    assert!(out_of_stack(script(&code, Language::JavaScript)));
    let code = format!("x = /{}/;", wrapped("(", "a", ")", 1_000));
    assert_eq!(script(&code, Language::JavaScript), Ok(()));
}

/// An attempt that may fail must not hold the error back and try the
/// other reading: each level would then read everything inside it
/// twice, and the work doubles with every level.
#[test]
fn running_out_of_stack_inside_an_attempted_reading_ends_the_parse() {
    let calls = format!("x = {};", wrapped("f<T>(", "1", ")", 2_000));
    assert!(out_of_stack(script(&calls, Language::TypeScript)));
    let arrows = format!("x = {};", wrapped("<T>(a = ", "1", ") => 1", 2_000));
    assert!(out_of_stack(script(&arrows, Language::TypeScript)));
}

#[test]
fn long_chains_within_reach_are_read() {
    for language in [Language::JavaScript, Language::TypeScript] {
        let else_ifs = format!("if(a){{}}{}", "else if(a){}".repeat(2_000));
        assert_eq!(script(&else_ifs, language), Ok(()));
        let blocks = wrapped("{", "", "}", 2_000);
        assert_eq!(script(&blocks, language), Ok(()));
        let ternaries = format!("x = {}c;", "a?b:".repeat(3_000));
        assert_eq!(script(&ternaries, language), Ok(()));
        let assignments = format!("{}1;", "a=".repeat(3_000));
        assert_eq!(script(&assignments, language), Ok(()));
        let sums = format!("x = a{};", "+a".repeat(5_000));
        assert_eq!(script(&sums, language), Ok(()));
    }
}

#[test]
fn chains_far_past_the_stack_are_an_error() {
    for language in [Language::JavaScript, Language::TypeScript] {
        let sums = format!("x = a{};", "+a".repeat(50_000));
        assert!(out_of_stack(script(&sums, language)));
        let ternaries = format!("x = {}c;", "a?b:".repeat(50_000));
        assert!(out_of_stack(script(&ternaries, language)));
        let nots = format!("x = {}a;", "!".repeat(50_000));
        assert!(out_of_stack(script(&nots, language)));
    }
}

/// acorn reads a member chain in a loop: its length is no nesting.
#[test]
fn a_member_chain_of_any_length_is_read() {
    for language in [Language::JavaScript, Language::TypeScript] {
        for link in [".b", "[0]", "?.b", "!"] {
            if link == "!" && language == Language::JavaScript {
                continue;
            }
            let code = format!("x = a{};", link.repeat(300_000));
            assert_eq!(script(&code, language), Ok(()), "{link}");
            let end = parse_expression_at(&code, 4, language);
            assert_eq!(end.map_err(|e| e.message), Ok(code.len() as u32 - 1));
        }
    }
}
