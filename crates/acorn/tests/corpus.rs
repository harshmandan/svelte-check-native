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

use svn_acorn::{Reading, SyntaxError};

fn got(result: Result<Option<u32>, SyntaxError>) -> String {
    match result {
        Ok(Some(end)) => format!("end {end}"),
        Ok(None) => "ok".to_string(),
        Err(error) => format!("{} @{}", error.message, error.pos),
    }
}

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

#[test]
fn port_agrees_with_acorn_on_its_own_tests() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("corpus/javascript.json")).unwrap_or_default();
    assert!(cases.len() > 2500, "the corpus is all there");
    let mut failures = Vec::new();
    for case in &cases {
        let code = case["code"].as_str().unwrap_or_default();
        let readings = [
            (
                "script",
                got(svn_acorn::parse(code, Reading::Script).map(|()| None)),
            ),
            (
                "module",
                got(svn_acorn::parse(code, Reading::Module).map(|()| None)),
            ),
            (
                "exprAt1",
                got(svn_acorn::parse_expression_at(code, 0).map(Some)),
            ),
            (
                "stmtAt1",
                got(svn_acorn::parse_statement_at(code, 0).map(Some)),
            ),
        ];
        for (reading, got) in readings {
            let Some(want) = want(&case[reading]) else {
                continue;
            };
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
    assert!(
        failures.is_empty(),
        "{} of the readings differ:\n{}",
        failures.len(),
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
