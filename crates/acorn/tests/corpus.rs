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

/// What acorn did. Where it failed with something other than a syntax
/// error at an offset, that failure's own message.
fn want(value: &serde_json::Value) -> Option<String> {
    if let Some(message) = value["crash"].as_str() {
        return Some(format!("crash {message}"));
    }
    Some(match (value["msg"].as_str(), value["end"].as_u64()) {
        (Some(message), _) => format!("{message} @{}", value["pos"]),
        (None, Some(end)) => format!("end {end}"),
        (None, None) => "ok".to_string(),
    })
}

/// What the port did with `text`, in the oracle's terms. `shift` is
/// how far into `text` the input starts: the oracle reads three of the
/// four ways one space into the text, and takes that space off again.
///
/// acorn-typescript raises a few errors at a column number that it
/// takes for an offset. The offset that names is worked out by the same
/// function the rest of the workspace reports those errors with.
fn got(text: &str, result: Result<Option<u32>, SyntaxError>, shift: u32) -> String {
    match result {
        Ok(Some(end)) => format!("end {}", end - shift),
        Ok(None) => "ok".to_string(),
        Err(error) if error.no_position => format!("crash {}", error.message),
        Err(error) => {
            let pos = if error.column_of {
                svn_acorn::column_as_offset(text, error.pos)
            } else {
                error.pos
            };
            format!("{} @{}", error.message, i64::from(pos) - i64::from(shift))
        }
    }
}

fn compare(corpus: &str, language: Language, at_least: usize) {
    let cases: Vec<serde_json::Value> = serde_json::from_str(corpus).unwrap_or_default();
    assert!(cases.len() > at_least, "the corpus is all there");
    compare_cases(&cases, language);
}

fn compare_cases(cases: &[serde_json::Value], language: Language) {
    let mut failures = Vec::new();
    let mut total = 0;
    for case in cases {
        let code = case["code"].as_str().unwrap_or_default();
        // As the oracle reads: a component's script, a tag's expression
        // or statement is never at offset zero of what the parser is
        // given, so one space stands for the rest.
        let padded = format!(" {code}");
        let readings = [
            (
                "script",
                got(
                    &padded,
                    svn_acorn::parse(&padded, Reading::Script, language).map(|()| None),
                    1,
                ),
            ),
            (
                "module",
                got(
                    code,
                    svn_acorn::parse(code, Reading::Module, language).map(|()| None),
                    0,
                ),
            ),
            (
                "exprAt1",
                got(
                    &padded,
                    svn_acorn::parse_expression_at(&padded, 1, language).map(Some),
                    1,
                ),
            ),
            (
                "stmtAt1",
                got(
                    &padded,
                    svn_acorn::parse_statement_at(&padded, 1, language).map(Some),
                    1,
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

/// A corpus from outside the repository: `ACORN_CORPUS_EXTRA` names a
/// file the oracle wrote, `ACORN_CORPUS_EXTRA_LANGUAGE` is `js` for
/// plain acorn's answers.
#[test]
fn port_agrees_on_a_corpus_given_by_the_environment() {
    let Ok(path) = std::env::var("ACORN_CORPUS_EXTRA") else {
        return;
    };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let cases: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap_or_default();
    assert!(!cases.is_empty(), "{path} holds a corpus");
    let language = match std::env::var("ACORN_CORPUS_EXTRA_LANGUAGE").as_deref() {
        Ok("js") => Language::JavaScript,
        _ => Language::TypeScript,
    };
    compare_cases(&cases, language);
}

/// The corpus says which parsers answered it, each by the commit of
/// its source. The port follows those commits; the script that writes
/// the corpus refuses any other checkout unless told to move the pins.
#[test]
fn corpus_names_the_commits_it_was_answered_from() {
    let pins: serde_json::Value =
        serde_json::from_str(include_str!("corpus/versions.json")).unwrap_or_default();
    for parser in ["acorn", "@sveltejs/acorn-typescript"] {
        let commit = pins[parser]["commit"].as_str().unwrap_or_default();
        assert!(
            commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()),
            "{parser} is pinned by a full commit hash, not {commit:?}"
        );
        assert!(
            pins[parser]["version"]
                .as_str()
                .is_some_and(|v| !v.is_empty()),
            "{parser} names the release its source is at or ahead of"
        );
    }
}
