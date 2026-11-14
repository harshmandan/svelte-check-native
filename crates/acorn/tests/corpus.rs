//! The port against acorn itself.
//!
//! `corpus/javascript.json` holds every input of acorn's own test
//! suite (the programs it must accept and the ones it must reject),
//! each with what acorn 8.19.0 does with it when read the four ways
//! the Svelte compiler reads text:
//!
//! - `script`: a component's `<script>`, never at offset zero, free to
//!   export a name it does not declare;
//! - `module`: a module file from its first character;
//! - `exprAt1`: `parseExpressionAt`, parentheses kept;
//! - `stmtAt1`: one statement, as the compiler reads a declaration tag.
//!
//! An entry is the error's message and byte offset, or where the
//! expression or statement ends, or nothing for a program read whole.
//! `scripts/acorn-corpus-oracle.mjs` writes the file from acorn.
//!
//! The port has to agree on every one. acorn decides which error comes
//! first and where it points by how it is written, so agreement on a
//! corpus this wide is what "the same parser" means here.

use svn_acorn::{Language, Reading, SyntaxError};

/// What acorn did, or `None` where it failed with something other
/// than a syntax error.
fn want(value: &serde_json::Value) -> Option<String> {
    if !value["crash"].is_null() {
        return None;
    }
    Some(match (value["msg"].as_str(), value["end"].as_u64()) {
        (Some(message), _) => format!("{message} @{}", value["pos"]),
        (None, Some(end)) => format!("end {end}"),
        (None, None) => "ok".to_string(),
    })
}

/// The offset acorn's error carries, given the port's.
///
/// acorn-typescript raises a few errors at a column number that it
/// takes for an offset. The corpus holds that number as acorn computed
/// it: in the text it was given, which for three of the readings is the
/// input after one space, and then moved back by that space.
fn reported_offset(code: &str, error: &SyntaxError, padded: bool) -> i64 {
    if !error.column_of {
        return i64::from(error.pos);
    }
    let before = &code[..error.pos as usize];
    let line_start = before
        .rfind(['\n', '\r', '\u{2028}', '\u{2029}'])
        .map(|i| i + before[i..].chars().next().map_or(1, char::len_utf8));
    let column = |from: usize| before[from..].encode_utf16().count() as i64;
    match (line_start, padded) {
        (Some(start), false) => column(start),
        (Some(start), true) => column(start) - 1,
        // On the first line the space counts, and is taken off again.
        (None, _) => column(0),
    }
}

fn got(code: &str, result: Result<Option<u32>, SyntaxError>, padded: bool) -> String {
    match result {
        Ok(Some(end)) => format!("end {end}"),
        Ok(None) => "ok".to_string(),
        Err(error) => format!(
            "{} @{}",
            error.message,
            reported_offset(code, &error, padded)
        ),
    }
}

fn compare(corpus: &str, language: Language, at_least: usize) {
    let cases: Vec<serde_json::Value> = serde_json::from_str(corpus).unwrap_or_default();
    assert!(cases.len() > at_least, "the corpus is all there");
    let mut failures = Vec::new();
    let mut total = 0;
    for case in &cases {
        let code = case["code"].as_str().unwrap_or_default();
        let readings = [
            (
                "script",
                got(
                    code,
                    svn_acorn::parse(code, Reading::Script, language).map(|()| None),
                    true,
                ),
            ),
            (
                "module",
                got(
                    code,
                    svn_acorn::parse(code, Reading::Module, language).map(|()| None),
                    false,
                ),
            ),
            (
                "exprAt1",
                got(
                    code,
                    svn_acorn::parse_expression_at(code, 0, language).map(Some),
                    true,
                ),
            ),
            (
                "stmtAt1",
                got(
                    code,
                    svn_acorn::parse_statement_at(code, 0, language).map(Some),
                    true,
                ),
            ),
        ];
        for (reading, got) in readings {
            let Some(want) = want(&case[reading]) else {
                continue;
            };
            total += 1;
            // Where acorn runs out of stack is up to the JavaScript
            // engine and differs from one run to the next; only that it
            // does is compared.
            const OUT_OF_STACK: &str = "Not enough stack space to parse input";
            if got.starts_with(OUT_OF_STACK) && want.starts_with(OUT_OF_STACK) {
                continue;
            }
            if got != want {
                let shown: String = code.chars().take(200).collect();
                failures.push(format!(
                    "{reading} {shown:?}\n  got:  {got}\n  want: {want}"
                ));
            }
        }
    }
    if let Ok(path) = std::env::var("ACORN_CORPUS_DIFF") {
        let _ = std::fs::write(path, failures.join("\n"));
    }
    assert!(
        failures.is_empty(),
        "{} of {total} readings differ:\n{}",
        failures.len(),
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn port_agrees_with_acorn_on_its_own_tests() {
    compare(
        include_str!("corpus/javascript.json"),
        Language::JavaScript,
        2500,
    );
}

#[test]
fn port_agrees_with_acorn_typescript() {
    compare(
        include_str!("corpus/typescript.json"),
        Language::TypeScript,
        3000,
    );
}
