//! Diagnostic filters — predicates that recognize false-positive
//! categories we drop in `map_diagnostic`.
//!
//! Most predicates take overlay text + byte offset. The byte offset
//! comes from [`crate::position::overlay_byte_offset`] in the
//! diagnostic mapper.

use std::path::Path;

use crate::types::{CheckDiagnostic, IGNORE_END_MARKER, IGNORE_START_MARKER};

/// SVELTE-4-COMPAT candidate: suppress TS2695 "Left side of comma
/// operator is unused and has no side effects" on `.svelte` files
/// that specifically trigger the Svelte-4 `$: (a, b, c)` dep-tracking
/// idiom. Upstream svelte-check filters these via
/// `isInReactiveStatement` in
/// `language-server/src/plugins/typescript/features/DiagnosticsProvider.ts:512-543`
/// — only diagnostics whose overlay AST node has a `$:` labeled-
/// statement ancestor get suppressed.
///
/// Historical note: this used to be a blanket drop of ALL TS2695 on
/// `.svelte` files. Empirical survey across our bench fleet found
/// exactly zero legitimate dep-tracking hits the blanket filter
/// silenced that weren't already silenced by emit rewrites, and ONE
/// upstream-matching fire it wrongly suppressed. The blanket filter
/// was removed in favour of this narrower, currently never-fires
/// path. Extend if a future Svelte-4 project surfaces the idiom.
pub(crate) fn is_svelte4_reactive_noop_comma(diag: &CheckDiagnostic) -> bool {
    let _ = diag;
    false
}

/// SVELTE-4-COMPAT: is the TS7028 ("Unused label") at overlay byte
/// `offset` the `$` label of a reactive statement?
///
/// Reactive statements reach the overlay as `$: …` labeled statements,
/// either directly in the render function's body or wrapped in an arrow
/// (`;() => { $: … }`) that sits directly in that body. Their `$` label is
/// never jumped to, so a tsconfig with `allowUnusedLabels: false` flags
/// every one. svelte-check drops exactly those: the flagged node must be
/// the label identifier of a `$` labeled statement whose parent chain is
/// the render function's body — or a block, an arrow function and an
/// expression statement in that body — with the render function declared
/// at the top of the file. A `$:` inside a user function is an ordinary
/// label there, and its TS7028 is reported.
pub(crate) fn is_reactive_statement_label(overlay: &str, is_ts: bool, offset: u32) -> bool {
    use oxc_ast::ast::{ArrowFunctionBody, Expression, FunctionBody, Statement};

    let is_dollar_label_at = |stmt: &Statement<'_>| {
        matches!(stmt, Statement::LabeledStatement(l)
            if l.label.name == "$" && l.label.span.start == offset)
    };
    let body_has_label = |body: &FunctionBody<'_>| {
        body.statements.iter().any(|stmt| {
            if is_dollar_label_at(stmt) {
                return true;
            }
            let Statement::ExpressionStatement(expr) = stmt else {
                return false;
            };
            let Expression::ArrowFunctionExpression(arrow) = &expr.expression else {
                return false;
            };
            matches!(&arrow.body, ArrowFunctionBody::FunctionBody(block)
                if block.statements.iter().any(is_dollar_label_at))
        })
    };

    let alloc = oxc_allocator::Allocator::default();
    let lang = if is_ts {
        svn_parser::ScriptLang::Ts
    } else {
        svn_parser::ScriptLang::Js
    };
    let parsed = svn_parser::parse_script_body(&alloc, overlay, lang);
    parsed.program.body.iter().any(|stmt| {
        let Statement::FunctionDeclaration(render) = stmt else {
            return false;
        };
        render
            .id
            .as_ref()
            .is_some_and(|id| id.name.starts_with(RENDER_FUNCTION_PREFIX))
            && render
                .body
                .as_ref()
                .is_some_and(|body| body_has_label(body))
    })
}

/// Name prefix of the component's render function in the overlay; the
/// emit crate appends a per-file hash.
const RENDER_FUNCTION_PREFIX: &str = "$$render_";

/// Upstream's `isInGeneratedCode` (`language-server/src/plugins/
/// typescript/features/utils.ts`), verbatim: a diagnostic spanning
/// overlay bytes `start..end` is generated when the nearest ignore-start
/// marker at or before `start` follows the nearest ignore-end marker
/// there (or that end marker is also the first one at or after `end`),
/// and an ignore-end marker follows. A position on the start marker
/// itself counts as generated.
pub(crate) fn is_in_generated_code(text: &str, start: usize, end: usize) -> bool {
    // `text.lastIndexOf(s, from)` / `text.indexOf(s, from)`, -1 for none.
    let last_index_of = |s: &str, from: usize| -> i64 {
        let limit = from.saturating_add(s.len()).min(text.len());
        text.get(..limit)
            .and_then(|head| head.rfind(s))
            .map_or(-1, |i| i as i64)
    };
    let index_of = |s: &str, from: usize| -> i64 {
        text.get(from.min(text.len())..)
            .and_then(|tail| tail.find(s))
            .map_or(-1, |i| (i + from) as i64)
    };
    let last_start = last_index_of(IGNORE_START_MARKER, start);
    let last_end = last_index_of(IGNORE_END_MARKER, start);
    let next_end = index_of(IGNORE_END_MARKER, end);
    (last_start > last_end || last_end == next_end) && last_start < next_end
}

/// Does the diagnostic at `offset` fall inside an
/// `__svn_ensure_transition(...)` call? Used to drop TS2554
/// "Expected N arguments" — emit wraps every `transition:` /
/// `in:` / `out:` directive call in `__svn_ensure_transition(...)`
/// to give tsgo a typed signature, but the inner user function
/// (e.g. `myTransition(node, params, context)`) declares the
/// optional 3rd `_context` parameter as required and tsgo fires
/// 2554 because we only pass 2 args at the synthetic call site.
/// Svelte's transition runtime supplies the 3rd arg at runtime —
/// the user's source is correct, the synthetic 2-arg call site is
/// the artefact.
///
/// Mirrors upstream svelte-check's `expectedTransitionThirdArgument`
/// filter at
/// `language-tools/packages/language-server/src/plugins/typescript/features/DiagnosticsProvider.ts:663-705`
/// (and the typescript-go provider's variant at
/// `plugins/typescript-go/features/DiagnosticsProvider.ts:1199-1230`).
/// The upstream filter consults the language service to confirm the
/// inner call's signature has exactly 3 non-optional parameters. When
/// no language service is available upstream falls back to matching the
/// diagnostic message text — the substring ` 3`, i.e. "Expected 3
/// arguments". We have no TS language service in our pipeline, so the
/// caller mirrors that no-language-service fallback: it pairs
/// [`is_expected_three_arguments_message`] with this structural origin
/// check. The check here only confirms the diagnostic originates
/// inside the wrapper — if the bytes immediately preceding `offset`
/// (after walking back through identifier characters) end with
/// `__svn_ensure_transition(`. The wrapper only wraps user-supplied
/// transition function calls, so the false-positive surface is narrow.
pub(crate) fn is_overlay_in_ensure_transition_call(overlay: &str, offset: u32) -> bool {
    const PREFIX: &[u8] = b"__svn_ensure_transition(";
    let bytes = overlay.as_bytes();
    let mut cursor = offset as usize;
    if cursor > bytes.len() {
        return false;
    }
    // Walk back through any identifier / whitespace characters
    // to find the start of the inner callee identifier. tsgo's
    // TS2554 may point at the function name (TypeScript >=5.4) or
    // at the open paren of the inner call.
    while cursor > 0 {
        let prev = bytes[cursor - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$' {
            cursor -= 1;
        } else {
            break;
        }
    }
    // Skip optional whitespace between the wrapper's `(` and the
    // inner identifier (cosmetic — emit doesn't insert any, but
    // future-proof against a formatter run).
    while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
        cursor -= 1;
    }
    if cursor < PREFIX.len() {
        return false;
    }
    &bytes[cursor - PREFIX.len()..cursor] == PREFIX
}

/// Does a TS2554 message describe the 3-argument transition contract
/// (`Expected 3 arguments, but got 2.`)?
///
/// Mirrors upstream's no-language-service fallback in
/// `expectedTransitionThirdArgument` verbatim — a `' 3'` substring
/// match on the flattened message. The synthetic wrapper call site
/// always passes exactly 2 args, so "but got 3" can never occur there
/// and the loose substring cannot false-match. A transition function
/// with 4+ required params produces "Expected 4 arguments, but got 2."
/// — no ` 3` — so its genuine arity error surfaces, matching the
/// typescript-go provider's exactly-3-non-optional-params signature
/// check.
pub(crate) fn is_expected_three_arguments_message(message: &str) -> bool {
    message.contains(" 3")
}

/// Rewrite a TS2322 that fired on an `inst.$$bindings = 'NAME'`
/// post-instance check into the user-facing "Cannot use 'bind:' with
/// this property" message. Returns `Some(new_message)` when the
/// diagnostic matches the shape, `None` to leave it untouched.
///
/// Mirrors upstream `moveBindingErrorMessage` (both the tsc and
/// typescript-go DiagnosticsProvider variants are identical): a
/// TS2322 whose flagged span text ends with `.$$bindings` is the
/// non-bindable-prop binding check. Upstream additionally walks the
/// Svelte AST to confirm an enclosing `InlineComponent` carries a
/// `Binding` attribute with the assigned name; we don't carry the
/// Svelte AST here, but our emit writes the `LHS.$$bindings = 'NAME'`
/// statement exclusively for a component's `bind:NAME` directive
/// (`emit_component_bindings_post_check`), so the span-suffix check
/// plus the quoted-name read establishes the same fact structurally.
///
/// Upstream also remaps the range onto the `bind:NAME` attribute —
/// ours arrives pre-mapped: the emit anchors both sides of the
/// assignment to the directive's source span via TokenMapEntries.
///
/// Message forms (verbatim upstream):
/// - When the original message follows the English `Type '"x"' is not
///   assignable to type '…'` shape, REPLACE it with the short form
///   naming the prop.
/// - Otherwise PREPEND the generic form, keeping the original text.
pub(crate) fn move_binding_error_message(
    overlay: &str,
    offset: u32,
    span: u32,
    message: &str,
) -> Option<String> {
    let start = offset as usize;
    let end = start.checked_add(span as usize)?;
    let flagged = overlay.get(start..end)?;
    if !flagged.ends_with(".$$bindings") {
        return None;
    }
    // The bound prop's name follows as `= 'NAME'` — read up to 100
    // bytes ahead (same window upstream uses) and take the first
    // single-quoted token. Bail when absent: not our emit shape.
    let tail = &overlay[end..(end + 100).min(overlay.len())];
    let quote = tail.find('\'')?;
    let name_rest = &tail[quote + 1..];
    let name_len = name_rest.find('\'')?;
    if name_len == 0 {
        return None;
    }

    const EXPLANATION: &str = "Cannot use 'bind:' with this property. \
         It is declared as non-bindable inside the component.\n";
    if message.starts_with("Type '") && message.contains("is not assignable to type '") {
        // `Type '"NAME"' is not assignable to …` — pull NAME out of
        // the message so the hint names the exact prop.
        if let Some(idx) = message.find("Type '\"") {
            let after = idx + "Type '\"".len();
            if let Some(name_end) = message[after..].find('"') {
                let prop = &message[after..after + name_end];
                return Some(format!(
                    "{EXPLANATION}To mark a property as bindable: \
                     'let {{ {prop} = $bindable() }} = $props()'"
                ));
            }
        }
    }
    Some(format!(
        "{EXPLANATION}To mark a property as bindable: \
         'let {{ prop = $bindable() }} = $props()'\n\n{message}"
    ))
}

/// Message-clarity adjustments for JSX-era / confusing TS nomenclature.
/// Mirrors upstream `adjustIfNecessary` (identical in the tsc and
/// typescript-go DiagnosticsProvider variants), which runs on every
/// Svelte-document diagnostic right before conversion:
///
/// - TS2345 mentioning `ConstructorOfATypedSvelteComponent` gains a
///   "Possible causes" trailer (with an extra SvelteComponentTyped
///   how-to on pre-5 Svelte).
/// - TS1184 ("Modifiers cannot appear here.") gains the
///   move-into-module-script hint.
///
/// `svelte5_plus` mirrors upstream's `isSvelte5Plus`
/// (`Number(version?.split('.')[0]) >= 5` — an UNKNOWN version is
/// falsy and gets the longer pre-5 text).
pub(crate) fn adjust_message_if_necessary(code: u32, message: &mut String, svelte5_plus: bool) {
    if code == 2345 && message.contains("ConstructorOfATypedSvelteComponent") {
        message.push_str(
            "\n\nPossible causes:\n\
             - You use the instance type of a component where you should use the constructor type\n\
             - Type definitions are missing for this Svelte Component. ",
        );
        if !svelte5_plus {
            message.push_str(
                "If you are using Svelte 3.31+, use SvelteComponentTyped to add a definition:\n  \
                 import type { SvelteComponentTyped } from \"svelte\";\n  \
                 class ComponentName extends SvelteComponentTyped<{propertyName: string;}> {}",
            );
        }
    }
    if code == 1184 {
        message.push_str(
            "\nIf this is a declare statement, move it into <script context=\"module\">..</script>",
        );
    }
}

/// Is the workspace's `svelte` package major version ≥ 5? Walks up
/// from `workspace` looking for `node_modules/svelte/package.json`
/// (same walk-up the engine discovery performs) and parses the
/// `"version"` major. Unknown (no svelte install, unreadable
/// manifest) reports `false`, mirroring upstream's
/// `isSvelte5Plus = Number(undefined) >= 5` fallthrough.
pub fn workspace_svelte_is_5_plus(workspace: &Path) -> bool {
    let mut dir = Some(workspace);
    while let Some(d) = dir {
        let manifest = d.join("node_modules").join("svelte").join("package.json");
        if let Ok(text) = std::fs::read_to_string(&manifest) {
            return svelte_manifest_major(&text).is_some_and(|major| major >= 5);
        }
        dir = d.parent();
    }
    false
}

/// Extract the major version from a package.json's `"version"` field.
/// Cheap textual scan — the manifest is npm-generated JSON and the
/// field value is always a plain semver string.
fn svelte_manifest_major(manifest: &str) -> Option<u32> {
    let idx = manifest.find("\"version\"")?;
    let rest = &manifest[idx + "\"version\"".len()..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find(['.', '"'])?;
    rest[..end].parse().ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn binding_error_message_rewrites_english_shape() {
        let overlay = "__svn_inst_0.$$bindings = 'notBindable';";
        let span = "__svn_inst_0.$$bindings".len() as u32;
        let msg = "Type '\"notBindable\"' is not assignable to type '\"bindableProp\"'.";
        let rewritten = move_binding_error_message(overlay, 0, span, msg).expect("rewrites");
        assert_eq!(
            rewritten,
            "Cannot use 'bind:' with this property. It is declared as non-bindable inside the component.\n\
             To mark a property as bindable: 'let { notBindable = $bindable() } = $props()'"
        );
    }

    #[test]
    fn binding_error_message_prepends_on_unrecognised_shape() {
        let overlay = "__svn_inst_0.$$bindings = 'x';";
        let span = "__svn_inst_0.$$bindings".len() as u32;
        let msg = "Der Typ ist nicht zuweisbar.";
        let rewritten = move_binding_error_message(overlay, 0, span, msg).expect("rewrites");
        assert!(rewritten.starts_with("Cannot use 'bind:' with this property."));
        assert!(rewritten.ends_with("\n\nDer Typ ist nicht zuweisbar."));
    }

    #[test]
    fn binding_error_message_ignores_other_2322_spans() {
        // A TS2322 whose span is NOT the $$bindings LHS stays untouched.
        let overlay = "const bad: string = 123;";
        assert!(move_binding_error_message(overlay, 6, 3, "Type 'number' ...").is_none());
        // ...as does a $$bindings span not followed by a quoted name.
        let overlay = "x.$$bindings = name;";
        assert!(move_binding_error_message(overlay, 0, 12, "Type ...").is_none());
    }

    #[test]
    fn adjust_message_appends_ts1184_hint() {
        let mut msg = String::from("Modifiers cannot appear here.");
        adjust_message_if_necessary(1184, &mut msg, true);
        assert_eq!(
            msg,
            "Modifiers cannot appear here.\nIf this is a declare statement, move it into <script context=\"module\">..</script>"
        );
    }

    #[test]
    fn adjust_message_appends_constructor_causes_variants() {
        let base = "Argument of type 'X' is not assignable to parameter of type 'ConstructorOfATypedSvelteComponent'.";
        let mut v5 = String::from(base);
        adjust_message_if_necessary(2345, &mut v5, true);
        assert!(v5.ends_with("- Type definitions are missing for this Svelte Component. "));
        let mut v4 = String::from(base);
        adjust_message_if_necessary(2345, &mut v4, false);
        assert!(v4.contains("SvelteComponentTyped to add a definition"));
        // Other codes / other messages are untouched.
        let mut other = String::from("Argument of type 'A' is not assignable ...");
        adjust_message_if_necessary(2345, &mut other, true);
        assert_eq!(other, "Argument of type 'A' is not assignable ...");
    }

    #[test]
    fn svelte_manifest_major_parses_semver() {
        assert_eq!(
            svelte_manifest_major(r#"{ "name": "svelte", "version": "5.56.5" }"#),
            Some(5)
        );
        assert_eq!(svelte_manifest_major(r#"{"version":"4.2.19"}"#), Some(4));
        assert_eq!(svelte_manifest_major(r#"{"name":"svelte"}"#), None);
    }

    /// Triage of upstream's `DiagnosticCode` enum
    /// (`DiagnosticsProvider.ts`). Every code upstream's diagnostic
    /// pipeline names must be accounted for here as one of:
    ///
    /// - `ported` — we implement the equivalent handling (filter,
    ///   message adjustment, or an emit shape that produces the same
    ///   observable result);
    /// - `not-applicable-cli` — the upstream handling can't fire on
    ///   the CLI path we mirror (lang-gated, LSP-only, dead code, or
    ///   made unreachable by our emit shape);
    /// - `tracked` — a known gap with a pointer to where it's
    ///   tracked.
    ///
    /// The test below parses the enum out of the pinned submodule, so
    /// a submodule bump that adds a new code fails here until the
    /// code is triaged into this table.
    const UPSTREAM_DIAGNOSTIC_CODE_TRIAGE: &[(u32, &str)] = &[
        (
            1184,
            "ported: adjust_message_if_necessary appends the move-into-module-script hint",
        ),
        (
            2454,
            "ported: map_diagnostic drops TS2454 whose flagged source text is a name the instance script exports",
        ),
        (
            2607,
            "not-applicable-cli: declared upstream but unused (JSX-era legacy)",
        ),
        (
            2786,
            "not-applicable-cli: declared upstream but unused (JSX-era legacy)",
        ),
        (
            2695,
            "not-applicable-cli: resolveNoopsInReactiveStatements needs the checker, which the --tsgo command never runs; the comma-operator diagnostics inside `$:` statements are reported as tsgo emits them",
        ),
        (
            6133,
            "ported: suggestion reclassification (include_suggestions) + pug-filter exception",
        ),
        (
            6192,
            "ported: suggestion reclassification + pug-filter exception",
        ),
        (
            7028,
            "ported: is_reactive_statement_label drops the `$` label of reactive statements in the render function",
        ),
        (
            17001,
            "not-applicable-cli: declared upstream but unused (JSX-era legacy)",
        ),
        (
            2300,
            "ported: template_nodes::is_element_attribute_name drops duplicates whose mapped start is an element attribute name",
        ),
        (
            1117,
            "ported: template_nodes::is_element_attribute_name drops duplicates whose mapped start is an element attribute name",
        ),
        (
            2345,
            "ported: adjust_message_if_necessary ConstructorOfATypedSvelteComponent trailer; the $store misuse enhancement is tracked (ls_diagnostics skip $store-wrong-usage)",
        ),
        (
            2322,
            "ported: move_binding_error_message rewrites the non-bindable bind: check",
        ),
        (
            2820,
            "not-applicable-cli: declared upstream but unused (JSX-era legacy)",
        ),
        (2353, "not-applicable-cli: declared upstream but unused"),
        (
            2739,
            "ported: emit's component-call TokenMapEntry anchors missing-prop diagnostics at the start tag (rangeMapper's getNodeIfIsInStartTag equivalent)",
        ),
        (
            2741,
            "ported: same component-call TokenMapEntry anchoring as 2739",
        ),
        (
            2769,
            "tracked: $store misuse enhancement not ported — ls_diagnostics skip $store-wrong-usage",
        ),
        (
            2304,
            "not-applicable-cli: used only by CodeActionsProvider (LSP quick fixes)",
        ),
        (
            2552,
            "not-applicable-cli: used only by CodeActionsProvider (LSP quick fixes)",
        ),
        (
            2554,
            "ported: is_overlay_in_ensure_transition_call drops the 3-arg transition contract case",
        ),
        (
            6387,
            "tracked: tsgo CLI emits no deprecation suggestions today — ls_diagnostics skip deprecated-unused-hints",
        ),
    ];

    #[test]
    fn every_upstream_diagnostic_code_is_triaged() {
        let upstream = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../language-tools/packages/language-server/src/plugins/typescript/features/DiagnosticsProvider.ts",
        );
        let Ok(text) = std::fs::read_to_string(&upstream) else {
            eprintln!(
                "SKIP: language-tools submodule not checked out at {}",
                upstream.display()
            );
            return;
        };
        let enum_start = text
            .find("enum DiagnosticCode")
            .expect("upstream DiagnosticCode enum moved — update this test");
        let mut upstream_codes: Vec<u32> = Vec::new();
        for line in text[enum_start..].lines() {
            // The enum closes with a lone `}` — comments inside it may
            // contain `}` (e.g. `'{0}'` placeholders), so scan by line
            // rather than `find('}')`.
            if line.trim() == "}" {
                break;
            }
            // Shape: `    NAME = 1234, // "..."`.
            let Some((_, rhs)) = line.split_once('=') else {
                continue;
            };
            let num: String = rhs
                .trim_start()
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if let Ok(code) = num.parse::<u32>() {
                upstream_codes.push(code);
            }
        }
        assert!(
            upstream_codes.len() >= 20,
            "parsed suspiciously few codes ({}) — enum shape changed?",
            upstream_codes.len()
        );
        for code in &upstream_codes {
            assert!(
                UPSTREAM_DIAGNOSTIC_CODE_TRIAGE
                    .iter()
                    .any(|(c, _)| c == code),
                "upstream DiagnosticCode {code} is not triaged — a submodule bump added \
                 a code the CLI pipeline hasn't decided on. Add it to \
                 UPSTREAM_DIAGNOSTIC_CODE_TRIAGE as ported / not-applicable-cli / tracked \
                 (with the matching implementation or tracking pointer)."
            );
        }
        // Inverse guard: triage entries that no longer exist upstream
        // are stale and should be removed.
        for (code, _) in UPSTREAM_DIAGNOSTIC_CODE_TRIAGE {
            assert!(
                upstream_codes.contains(code),
                "triage entry {code} no longer exists in upstream's DiagnosticCode enum — remove it"
            );
        }
    }
}
