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

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

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
];

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct ExpectedWarning {
    code: String,
    message: String,
    start: LineCol,
    end: LineCol,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
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
    if !dir.is_dir() {
        eprintln!(
            "SKIP: upstream clone not available at {}. Run \
             `git clone --filter=blob:none --no-checkout \
             https://github.com/sveltejs/svelte.git .svelte-upstream/svelte && \
             git -C .svelte-upstream/svelte checkout HEAD -- \
             packages/svelte/messages packages/svelte/tests/validator` to enable.",
            dir.display()
        );
        return;
    }

    let ported: BTreeSet<&str> = PORTED_CODES.iter().copied().collect();
    let mut total = 0usize;
    let mut enforced = 0usize;
    let mut passing = 0usize;
    let mut skipped = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut uncovered: BTreeSet<String> = BTreeSet::new();

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

        let source_path = if sample_path.join("input.svelte").is_file() {
            sample_path.join("input.svelte")
        } else if sample_path.join("input.svelte.js").is_file() {
            // Module-only source — Phase A can't lint these yet (JS AST
            // pass lands Phase C). Skip for now.
            skipped += 1;
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
            skipped += 1;
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
            skipped += 1;
            for w in &expected {
                if !ported.contains(w.code.as_str()) {
                    uncovered.insert(w.code.clone());
                }
            }
            continue;
        }
        enforced += 1;

        // Run our linter.
        let warnings = svn_lint::lint_file(
            &source,
            &source_path,
            None,
            svn_lint::CompatFeatures::MODERN,
        );
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

    eprintln!("upstream validator fixtures:");
    eprintln!("  total with warnings.json: {total}");
    eprintln!("  enforced (all codes ported): {enforced}");
    eprintln!("  passing: {passing}");
    eprintln!("  skipped (unported code in fixture): {skipped}");
    if !uncovered.is_empty() {
        eprintln!(
            "  codes not yet ported (at least one fixture blocked):\n    {}",
            uncovered.iter().cloned().collect::<Vec<_>>().join("\n    ")
        );
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
