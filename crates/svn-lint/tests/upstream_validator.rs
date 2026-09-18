#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Integration test: run our linter against upstream's
//! `packages/svelte/tests/validator/samples/` fixtures.
//!
//! Upstream fixture shape:
//! ```text
//! fixture_name/
//!   input.svelte              ← source
//!   warnings.json             ← [{code, message, start:{line,col}, end:{line,col}}]
//!   _config.js  (optional)    ← sometimes sets compileOptions
//!   options.json (optional)   ← JSON-form options
//! ```
//!
//! Upstream's `test.ts:21` strips the trailing `\nhttps://svelte.dev/e/...`
//! from `w.message` before deepEqualing; expected messages in the
//! JSON are the plain template form only.
//!
//! Line is 1-based, column is 0-based (acorn/locate-character convention).
//!
//! **Gating strategy.** At Phase 0 almost every fixture fails — we
//! haven't implemented most rules. Tests opt fixtures in via
//! `PORTED_CODES` — a fixture passes the gate if *every* expected
//! code is in that set AND our output matches exactly. Fixtures with
//! any unported code are marked `skipped` (printed but not failed).
//! When a code is added to `PORTED_CODES`, its fixtures start
//! enforcing; regressions fail loudly.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Codes for which we've implemented the rule and are ready to
/// enforce upstream-fixture parity.
///
/// **Extend this list as each rule lands.** The test runner only
/// fails on fixtures whose expected codes are ALL in this set.
const PORTED_CODES: &[&str] = &[
    "element_invalid_self_closing_tag",
    "attribute_illegal_colon",
    "attribute_avoid_is",
    "attribute_invalid_property_name",
    "attribute_quoted",
    "block_empty",
    "event_directive_deprecated",
    "svelte_component_deprecated",
    "svelte_self_deprecated",
    "script_unknown_attribute",
    "script_context_deprecated",
    "slot_element_deprecated",
    "perf_avoid_inline_class",
    "perf_avoid_nested_class",
    "reactive_declaration_invalid_placement",
    "node_invalid_placement_ssr",
    "component_name_lowercase",
    "attribute_global_event_reference",
    "state_referenced_locally",
    "non_reactive_update",
    "reactive_declaration_module_script_dependency",
    "store_rune_conflict",
    "legacy_component_creation",
    "bidirectional_control_characters",
    "bind_invalid_each_rest",
    "export_let_unused",
    "a11y_accesskey",
    "a11y_autofocus",
    "a11y_distracting_elements",
    "a11y_positive_tabindex",
    "a11y_misplaced_scope",
    "a11y_missing_attribute",
    "a11y_img_redundant_alt",
    "a11y_missing_content",
    "a11y_hidden",
    "a11y_consider_explicit_label",
    "a11y_label_has_associated_control",
    "a11y_media_has_caption",
    "a11y_figcaption_parent",
    "a11y_figcaption_index",
    "a11y_invalid_attribute",
    "a11y_aria_attributes",
    "a11y_unknown_aria_attribute",
    "a11y_incorrect_aria_attribute_type",
    "a11y_incorrect_aria_attribute_type_boolean",
    "a11y_incorrect_aria_attribute_type_idlist",
    "a11y_incorrect_aria_attribute_type_integer",
    "a11y_incorrect_aria_attribute_type_token",
    "a11y_incorrect_aria_attribute_type_tokenlist",
    "a11y_incorrect_aria_attribute_type_tristate",
    "a11y_misplaced_role",
    "a11y_aria_activedescendant_has_tabindex",
    "a11y_no_abstract_role",
    "a11y_unknown_role",
    "a11y_no_redundant_roles",
    "a11y_no_noninteractive_tabindex",
    "a11y_click_events_have_key_events",
    "a11y_mouse_events_have_key_events",
    "a11y_role_has_required_aria_props",
    "a11y_role_supports_aria_props",
    "a11y_role_supports_aria_props_implicit",
    "a11y_no_interactive_element_to_noninteractive_role",
    "a11y_no_noninteractive_element_to_interactive_role",
    "a11y_no_static_element_interactions",
    "a11y_no_noninteractive_element_interactions",
    "a11y_interactive_supports_focus",
    "legacy_code",
    "unknown_code",
    "a11y_autocomplete_valid",
    "custom_element_props_identifier",
    "options_missing_custom_element",
    "element_implicitly_closed",
    "options_deprecated_accessors",
    "options_deprecated_immutable",
    "svelte_element_invalid_this",
];

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
struct ExpectedWarning {
    code: String,
    message: String,
    start: LineCol,
    end: LineCol,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
struct LineCol {
    line: u32,
    column: u32,
}

fn strip_link(message: &str) -> &str {
    match message.rfind('\n') {
        Some(i) => &message[..i],
        None => message,
    }
}

fn validator_samples_dir() -> PathBuf {
    // A checkout whose submodule is not initialised (e.g. a secondary git
    // worktree) can point at another clone of the svelte repo instead.
    if let Some(root) = std::env::var_os("SVELTE_UPSTREAM_DIR") {
        return PathBuf::from(root).join("packages/svelte/tests/validator/samples");
    }
    // tests/ runs under the crate dir; reach the workspace root.
    let manifest = env!("CARGO_MANIFEST_DIR");
    let ws = PathBuf::from(manifest)
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    ws.join(".svelte-upstream/svelte/packages/svelte/tests/validator/samples")
}

#[test]
fn upstream_validator_fixtures() {
    let dir = validator_samples_dir();
    assert!(
        dir.is_dir(),
        "upstream clone not available at {}. \
         The svelte-upstream submodule is required: run \
         `git submodule update --init --recursive .svelte-upstream/svelte` \
         from the workspace root.",
        dir.display()
    );

    let ported: BTreeSet<&str> = PORTED_CODES.iter().copied().collect();
    let mut total = 0usize;
    let mut enforced = 0usize;
    let mut passing = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut uncovered: BTreeSet<String> = BTreeSet::new();
    // Skip reasons broken out so the scoreboard reports the long-tail
    // bucket each skip falls in. Treat each bucket as an explicit
    // backlog item — module-mode samples ride on Phase C JS-pass,
    // compileOption-gated ones ride on a future filter surface, etc.
    let mut skipped_module_mode: Vec<String> = Vec::new();
    let mut skipped_compile_options: Vec<String> = Vec::new();
    let mut skipped_unported_code: Vec<String> = Vec::new();

    for entry in fs::read_dir(&dir).unwrap() {
        let sample = entry.unwrap();
        let sample_path = sample.path();
        if !sample_path.is_dir() {
            continue;
        }
        let expected_path = sample_path.join("warnings.json");
        if !expected_path.is_file() {
            continue; // error-only fixture
        }

        total += 1;

        let fixture_name = sample_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        let source_path = if sample_path.join("input.svelte").is_file() {
            sample_path.join("input.svelte")
        } else if sample_path.join("input.svelte.js").is_file() {
            // Module-only source — Phase A can't lint these yet (JS AST
            // pass lands Phase C). Skip for now.
            skipped_module_mode.push(fixture_name);
            continue;
        } else {
            continue;
        };

        // Upstream occasionally ships a fixture behind `skip: true`
        // in its `_config.js` — the JS test runner doesn't execute
        // those. Mirror that so we don't unnecessarily enforce a
        // fixture upstream itself doesn't run. Also skip fixtures
        // that exercise `warningFilter` / `compileOptions` compile
        // options — our linter runs without those, so its output
        // can't match without wiring a filter surface (future work).
        let config_path = sample_path.join("_config.js");
        if config_path.is_file()
            && let Ok(cfg) = fs::read_to_string(&config_path)
            && (cfg.contains("skip: true")
                || cfg.contains("warningFilter")
                || cfg.contains("customElement")
                || cfg.contains("immutable"))
        {
            skipped_compile_options.push(fixture_name);
            continue;
        }

        let raw_source = fs::read_to_string(&source_path).unwrap();
        // Upstream's suite.ts strips trailing whitespace and normalises \r\n
        // before compiling. Mirror it for byte parity.
        let source = raw_source.trim_end().replace('\r', "");

        let expected: Vec<ExpectedWarning> =
            serde_json::from_str(&fs::read_to_string(&expected_path).unwrap()).unwrap();

        // Gate: only enforce if every expected code is in PORTED_CODES.
        let all_ported = expected.iter().all(|w| ported.contains(w.code.as_str()));
        if !all_ported {
            skipped_unported_code.push(fixture_name);
            for w in &expected {
                if !ported.contains(w.code.as_str()) {
                    uncovered.insert(w.code.clone());
                }
            }
            continue;
        }
        enforced += 1;

        // Run our linter.
        // The compiler runs these samples without a file name, which it
        // reports as `(unknown)`; module samples keep theirs so the
        // `.svelte.js` extension still selects runes mode.
        let lint_path = if source_path.extension().is_some_and(|e| e == "svelte") {
            std::path::PathBuf::from("(unknown)")
        } else {
            source_path.clone()
        };
        let warnings =
            svn_lint::lint_file(&source, &lint_path, None, svn_lint::CompatFeatures::MODERN);
        // Upstream emits line-1-based, column-0-based; we store line
        // 1-based and column 0-based in LintContext::emit.
        let actual: Vec<ExpectedWarning> = warnings
            .into_iter()
            .map(|w| ExpectedWarning {
                code: w.code.as_str().to_string(),
                message: strip_link(&w.message).to_string(),
                start: LineCol {
                    line: w.start_line,
                    column: w.start_column,
                },
                end: LineCol {
                    line: w.end_line,
                    column: w.end_column,
                },
            })
            .collect();

        if actual == expected {
            passing += 1;
        } else {
            failures.push(format!(
                "fixture {name}:\n  expected: {exp:#?}\n  actual:   {act:#?}",
                name = sample_path.file_name().unwrap().to_string_lossy(),
                exp = expected,
                act = actual,
            ));
        }
    }

    let total_skipped =
        skipped_module_mode.len() + skipped_compile_options.len() + skipped_unported_code.len();
    eprintln!("upstream validator fixtures:");
    eprintln!("  total with warnings.json: {total}");
    eprintln!("  enforced (all codes ported): {enforced}");
    eprintln!("  passing: {passing}");
    eprintln!("  skipped (total): {total_skipped}");
    eprintln!(
        "    - module-only (input.svelte.js): {}",
        skipped_module_mode.len()
    );
    eprintln!(
        "    - compile-option-gated (_config.js): {}",
        skipped_compile_options.len()
    );
    eprintln!(
        "    - unported code in expected output: {}",
        skipped_unported_code.len()
    );
    if !skipped_module_mode.is_empty() {
        let mut names = skipped_module_mode.clone();
        names.sort();
        eprintln!(
            "      module-only fixtures:\n        {}",
            names.join("\n        ")
        );
    }
    if !skipped_compile_options.is_empty() {
        let mut names = skipped_compile_options.clone();
        names.sort();
        eprintln!(
            "      compile-option-gated fixtures:\n        {}",
            names.join("\n        ")
        );
    }
    if !skipped_unported_code.is_empty() {
        let mut names = skipped_unported_code.clone();
        names.sort();
        eprintln!(
            "      unported-code fixtures:\n        {}",
            names.join("\n        ")
        );
    }
    if !uncovered.is_empty() {
        eprintln!(
            "  codes not yet ported (at least one fixture blocked):\n    {}",
            uncovered.iter().cloned().collect::<Vec<_>>().join("\n    ")
        );
    }

    if let Ok(path) = std::env::var("LINT_COVERAGE_REPORT") {
        let mut module_only = skipped_module_mode.clone();
        module_only.sort();
        let mut compile_option_gated = skipped_compile_options.clone();
        compile_option_gated.sort();
        let mut unported_code = skipped_unported_code.clone();
        unported_code.sort();
        let report = serde_json::json!({
            "submodule_sha": std::env::var("SVELTE_UPSTREAM_SHA").unwrap_or_default(),
            "total_with_warnings_json": total,
            "enforced": enforced,
            "passing": passing,
            "failing": enforced - passing,
            "skipped": {
                "module_only": module_only,
                "compile_option_gated": compile_option_gated,
                "unported_code": unported_code,
            },
            "ported_codes": ported.iter().copied().collect::<Vec<_>>(),
            "uncovered_codes": uncovered,
        });
        let pretty = serde_json::to_string_pretty(&report).unwrap();
        fs::write(&path, format!("{pretty}\n"))
            .unwrap_or_else(|e| panic!("failed to write coverage report to {path}: {e}"));
        eprintln!("  wrote coverage report → {path}");
    }

    assert!(
        failures.is_empty(),
        "{} failures among {} enforced fixtures:\n\n{}",
        failures.len(),
        enforced,
        failures.join("\n\n")
    );
}

fn _touch_path_unused(_: &Path) {}

/// One diagnostic we produced, in the same shape as the expected
/// entry plus the error/warning severity bit, so the report can tell
/// "we fired the right code as a warning" from "we fired it as an
/// error".
#[derive(Debug, Clone, Serialize)]
struct EmittedDiagnostic {
    code: String,
    is_error: bool,
    message: String,
    start: LineCol,
    end: LineCol,
}

/// How one `errors.json` sample compares against our output.
///
/// The compiler throws on its first error, so `errors.json` holds at
/// most one entry; an empty array means the sample compiles clean
/// (its warnings, if any, are asserted by the warnings test). When a
/// file has a compile error, svelte-check reports only that error and
/// none of the file's warnings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum ErrorSampleStatus {
    /// Exactly one error from us, equal to the expected one, no warnings.
    Match,
    /// The expected error matched but we also produced extra output
    /// (warnings upstream would suppress, or a second error).
    MatchExtraOutput,
    /// Same code, same range, different message text.
    WrongMessage,
    /// Same code, different range.
    WrongPosition,
    /// We produced an error, but never the expected code.
    WrongCode,
    /// Expected an error; we produced none.
    Missing,
    /// Expected `[]`; we produced an error.
    Spurious,
    /// Expected `[]`; we produced no error.
    Clean,
    /// `input.svelte.js` sample — module-only sources aren't linted.
    SkippedModuleOnly,
    /// `_config.js` uses compile options the linter doesn't model.
    SkippedCompileOptions,
}

#[derive(Debug, Serialize)]
struct ErrorSampleReport {
    name: String,
    status: ErrorSampleStatus,
    expected: Vec<ExpectedWarning>,
    ours: Vec<EmittedDiagnostic>,
    /// Warnings we emitted on a sample upstream rejects with a compile
    /// error — svelte-check would show none of these.
    warnings_upstream_suppresses: Vec<EmittedDiagnostic>,
}

fn classify_error_sample(
    expected: &[ExpectedWarning],
    ours: &[EmittedDiagnostic],
) -> ErrorSampleStatus {
    let our_errors: Vec<&EmittedDiagnostic> = ours.iter().filter(|d| d.is_error).collect();
    let Some(exp) = expected.first() else {
        return if our_errors.is_empty() {
            ErrorSampleStatus::Clean
        } else {
            ErrorSampleStatus::Spurious
        };
    };
    if our_errors.is_empty() {
        return ErrorSampleStatus::Missing;
    }
    let equal = |d: &EmittedDiagnostic| {
        d.code == exp.code && d.message == exp.message && d.start == exp.start && d.end == exp.end
    };
    if our_errors.iter().any(|d| equal(d)) {
        return if ours.len() == 1 {
            ErrorSampleStatus::Match
        } else {
            ErrorSampleStatus::MatchExtraOutput
        };
    }
    let same_code: Vec<&&EmittedDiagnostic> =
        our_errors.iter().filter(|d| d.code == exp.code).collect();
    if same_code.is_empty() {
        return ErrorSampleStatus::WrongCode;
    }
    if same_code
        .iter()
        .any(|d| d.start == exp.start && d.end == exp.end)
    {
        ErrorSampleStatus::WrongMessage
    } else {
        ErrorSampleStatus::WrongPosition
    }
}

/// Report-only survey of upstream's `errors.json` samples: how many of
/// the compiler's *error* codes our linter reproduces. Never fails;
/// writes a JSON report to `$LINT_ERRORS_REPORT` when set.
#[test]
fn upstream_validator_error_fixtures() {
    let dir = validator_samples_dir();
    assert!(
        dir.is_dir(),
        "upstream clone not available at {}. \
         The svelte-upstream submodule is required: run \
         `git submodule update --init --recursive .svelte-upstream/svelte` \
         from the workspace root.",
        dir.display()
    );

    let mut samples: Vec<ErrorSampleReport> = Vec::new();

    for entry in fs::read_dir(&dir).unwrap() {
        let sample_path = entry.unwrap().path();
        if !sample_path.is_dir() {
            continue;
        }
        let expected_path = sample_path.join("errors.json");
        if !expected_path.is_file() {
            continue;
        }
        let name = sample_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let expected: Vec<ExpectedWarning> =
            serde_json::from_str(&fs::read_to_string(&expected_path).unwrap()).unwrap();
        assert!(
            expected.len() <= 1,
            "{name}: errors.json has {} entries; the compiler throws on the first error",
            expected.len()
        );

        let skipped = |status| ErrorSampleReport {
            name: name.clone(),
            status,
            expected: expected.clone(),
            ours: Vec::new(),
            warnings_upstream_suppresses: Vec::new(),
        };

        let source_path = sample_path.join("input.svelte");
        if !source_path.is_file() {
            // Same gate as the warnings test: module-only sources
            // (`input.svelte.js`) aren't linted.
            samples.push(skipped(ErrorSampleStatus::SkippedModuleOnly));
            continue;
        }
        // Same compile-option gate as the warnings test.
        let config_path = sample_path.join("_config.js");
        if config_path.is_file()
            && let Ok(cfg) = fs::read_to_string(&config_path)
            && (cfg.contains("skip: true")
                || cfg.contains("warningFilter")
                || cfg.contains("customElement")
                || cfg.contains("immutable"))
        {
            samples.push(skipped(ErrorSampleStatus::SkippedCompileOptions));
            continue;
        }

        let raw_source = fs::read_to_string(&source_path).unwrap();
        let source = raw_source.trim_end().replace('\r', "");

        let ours: Vec<EmittedDiagnostic> = svn_lint::lint_file(
            &source,
            &source_path,
            None,
            svn_lint::CompatFeatures::MODERN,
        )
        .into_iter()
        .map(|w| EmittedDiagnostic {
            code: w.code.as_str().to_string(),
            is_error: w.is_error,
            message: strip_link(&w.message).to_string(),
            start: LineCol {
                line: w.start_line,
                column: w.start_column,
            },
            end: LineCol {
                line: w.end_line,
                column: w.end_column,
            },
        })
        .collect();

        let status = classify_error_sample(&expected, &ours);
        let warnings_upstream_suppresses = if expected.is_empty() {
            Vec::new()
        } else {
            ours.iter().filter(|d| !d.is_error).cloned().collect()
        };
        samples.push(ErrorSampleReport {
            name,
            status,
            expected,
            ours,
            warnings_upstream_suppresses,
        });
    }
    samples.sort_by(|a, b| a.name.cmp(&b.name));

    let mut by_status: BTreeMap<ErrorSampleStatus, usize> = BTreeMap::new();
    // code → (samples, matched)
    let mut by_code: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for s in &samples {
        *by_status.entry(s.status).or_default() += 1;
        if let Some(exp) = s.expected.first() {
            let slot = by_code.entry(exp.code.clone()).or_default();
            slot.0 += 1;
            if s.status == ErrorSampleStatus::Match {
                slot.1 += 1;
            }
        }
    }
    let suppressed: Vec<&ErrorSampleReport> = samples
        .iter()
        .filter(|s| !s.warnings_upstream_suppresses.is_empty())
        .collect();

    eprintln!("upstream validator error fixtures (report-only):");
    eprintln!("  total with errors.json: {}", samples.len());
    for (status, n) in &by_status {
        eprintln!("  {status:?}: {n}");
    }
    eprintln!(
        "  samples with warnings upstream would suppress: {}",
        suppressed.len()
    );
    let mut codes: Vec<(&String, &(usize, usize))> = by_code.iter().collect();
    codes.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
    eprintln!("  per expected code (samples / matched):");
    for (code, (total, matched)) in &codes {
        eprintln!("    {code}: {total} / {matched}");
    }

    if let Ok(path) = std::env::var("LINT_ERRORS_REPORT") {
        let report = serde_json::json!({
            "submodule_sha": std::env::var("SVELTE_UPSTREAM_SHA").unwrap_or_default(),
            "total_with_errors_json": samples.len(),
            "by_status": by_status,
            "by_code": codes
                .iter()
                .map(|(code, (total, matched))| {
                    serde_json::json!({"code": code, "samples": total, "matched": matched})
                })
                .collect::<Vec<_>>(),
            "warnings_upstream_suppresses": suppressed
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "name": s.name,
                        "status": s.status,
                        "warnings": s.warnings_upstream_suppresses,
                    })
                })
                .collect::<Vec<_>>(),
            "samples": samples,
        });
        let pretty = serde_json::to_string_pretty(&report).unwrap();
        fs::write(&path, format!("{pretty}\n"))
            .unwrap_or_else(|e| panic!("failed to write errors report to {path}: {e}"));
        eprintln!("  wrote errors report → {path}");
    }
}
